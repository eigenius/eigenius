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

//! Substrate boundary check per [D26 §7.5](../../../../docs/design/d26-runtime-substrate.md).
//!
//! Every `RunRuntimeScript` / `CallRuntimeMethod` dispatch passes
//! through this check before reaching the worker:
//!
//! 1. **Mirror anchor is ancestral.** The mirror's `source_layer`
//!    must be ancestral-to-or-equal-with the invocation's claim
//!    layer. Otherwise → [`RunError::MirrorAnchorNotAncestral`].
//! 2. **Required classes are mirrored.** Every class IRI the script
//!    declares in `requires_mirror_classes` (or every input/output
//!    type for a method signature) must appear in the mirror's
//!    `mirrored_classes`. Otherwise → [`RunError::MissingMirrorStruct`].
//! 3. **Mirrored classes are unchanged.** Each required class must
//!    be byte-identical between the mirror's anchor layer and the
//!    claim layer. Otherwise → [`RunError::MirrorVersionMismatch`].
//!
//! The check is intentionally chain-agnostic: it talks to a
//! [`ChainAccessor`] for all layer-related queries, so the substrate
//! crate stays testable with synthetic chains. The substrate's
//! orchestrator-side path uses this version (post-Phase-18b kernel
//! wiring; until then the kernel runs its own copy pre-dispatch).
//!
//! ## ⚠️ Keep this in sync with `kernel/src/runtime/boundary.rs`
//!
//! The kernel ships a parallel implementation of the same D26 §7.5
//! check that operates directly on `&Layer` and runs pre-dispatch.
//! The two files share property-IRI constants, the three-rule order,
//! the empty-mirror short-circuit, and the failure taxonomy (with
//! different variant names: substrate uses [`RunError`], kernel uses
//! `BoundaryError`).
//!
//! Why duplicate? The substrate crate already depends on the kernel
//! for `Resource`/`Iri`, so a unified check would need both to depend
//! on a third "boundary" crate that owns those types — a non-trivial
//! restructure for ~150 lines of mirrored logic. Revisit if a third
//! caller emerges.
//!
//! When you change one, change the other. If the check shape can
//! diverge (e.g. the kernel side gains a defense-in-depth rule the
//! substrate side doesn't need), document the divergence in both
//! files.

use crate::chain::ChainAccessor;
use crate::error::RunError;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};

// Property IRIs the boundary check reads. Statically known so error
// reporting can point at stable names.
const PROP_REQUIRES_ENVIRONMENT: &str = "urn:eigenius:runtime:requires_environment";
const PROP_REQUIRES_MIRROR_CLASSES: &str = "urn:eigenius:runtime:requires_mirror_classes";
const PROP_MIRROR_DEPENDENCY: &str = "urn:eigenius:runtime:mirror_dependency";
const PROP_SOURCE_LAYER: &str = "urn:eigenius:runtime:source_layer";
const PROP_MIRRORED_CLASSES: &str = "urn:eigenius:runtime:mirrored_classes";
const PROP_INPUT_TYPES: &str = "urn:eigenius:runtime:input_types";
const PROP_OUTPUT_TYPE: &str = "urn:eigenius:runtime:output_type";
const PROP_METHOD_PACKAGE: &str = "urn:eigenius:runtime:method_package";

/// Run the boundary check for a `RunRuntimeScript` invocation.
///
/// `script` is the `RuntimeScript` resource being dispatched.
/// `claim_layer` is the layer the invocation will commit its output
/// against — typically the head of the kernel's `ExecutionContext`.
/// `chain` provides read access to the layer chain.
pub fn check_run_script(
    script: &Resource,
    claim_layer: &Iri,
    chain: &dyn ChainAccessor,
) -> Result<(), RunError> {
    let env_iri = read_iri_property(script, PROP_REQUIRES_ENVIRONMENT)?;
    let required = read_iri_array(script, PROP_REQUIRES_MIRROR_CLASSES);
    run_check(&required, &env_iri, claim_layer, chain)
}

/// Run the boundary check for a `CallRuntimeMethod` invocation.
///
/// `signature` is the `RuntimeMethodSignature` resource. `env_iri`
/// is resolved by the caller — typically by walking
/// `signature.method_package` to the package's environment, or by
/// the substrate facade taking it explicitly. The signature's
/// `input_types` and `output_type` together form the set of classes
/// that must be mirrored.
pub fn check_call_method(
    signature: &Resource,
    env_iri: &Iri,
    claim_layer: &Iri,
    chain: &dyn ChainAccessor,
) -> Result<(), RunError> {
    let mut required = read_iri_array(signature, PROP_INPUT_TYPES);
    if let Some(out) = read_optional_iri_property(signature, PROP_OUTPUT_TYPE) {
        // Avoid duplicate work if the output type happens to be in
        // the input list too.
        if !required.contains(&out) {
            required.push(out);
        }
    }
    run_check(&required, env_iri, claim_layer, chain)
}

/// Common body of both check functions. Resolves the environment +
/// mirror, walks the required-class set, applies the three checks.
fn run_check(
    required: &[Iri],
    env_iri: &Iri,
    claim_layer: &Iri,
    chain: &dyn ChainAccessor,
) -> Result<(), RunError> {
    let env = chain.resolve(claim_layer, env_iri).ok_or_else(|| {
        boundary_failure(format!(
            "RuntimeEnvironment `{env_iri}` not found in chain at claim layer `{claim_layer}`"
        ))
    })?;

    // Envs without a mirror are valid (e.g. test-runtime envs without
    // typed Eigon dispatch). The caller has nothing to check.
    let mirror_iri = match read_optional_iri_property(&env, PROP_MIRROR_DEPENDENCY) {
        Some(iri) => iri,
        None => return Ok(()),
    };

    let mirror = chain.resolve(claim_layer, &mirror_iri).ok_or_else(|| {
        boundary_failure(format!(
            "RuntimePackageMirror `{mirror_iri}` not found in chain at claim layer `{claim_layer}`"
        ))
    })?;

    let mirror_layer = read_iri_property(&mirror, PROP_SOURCE_LAYER)?;

    // Check 1: mirror anchor is ancestral-or-equal.
    if !chain.is_ancestor_or_equal(&mirror_layer, claim_layer) {
        return Err(RunError::MirrorAnchorNotAncestral {
            mirror_layer: mirror_layer.as_str().to_string(),
            claim_layer: claim_layer.as_str().to_string(),
        });
    }

    let mirrored = read_iri_array(&mirror, PROP_MIRRORED_CLASSES);

    // Checks 2 & 3: each required class is mirrored AND unchanged.
    for class_iri in required {
        if !mirrored.contains(class_iri) {
            return Err(RunError::MissingMirrorStruct {
                class_iri: class_iri.as_str().to_string(),
            });
        }
        if !chain.class_unchanged_between(&mirror_layer, claim_layer, class_iri) {
            return Err(RunError::MirrorVersionMismatch {
                class_iri: class_iri.as_str().to_string(),
                mirror_layer: mirror_layer.as_str().to_string(),
                claim_layer: claim_layer.as_str().to_string(),
            });
        }
    }

    Ok(())
}

/// Wrap a boundary-side malformation as `MethodSignatureMismatch`.
/// The variant name is slightly off — it covers the broader category
/// "argument shape doesn't match what the boundary check expects" —
/// but it's the closest existing fit and avoids adding a fourth
/// variant whose only callers are this module.
fn boundary_failure(msg: String) -> RunError {
    RunError::MethodSignatureMismatch(msg)
}

fn read_iri_property(r: &Resource, prop_iri: &str) -> Result<Iri, RunError> {
    let prop = Iri::parse(prop_iri).expect("static IRI is well-formed");
    let value = r
        .get(&prop)
        .ok_or_else(|| boundary_failure(format!("missing required property `{prop_iri}`")))?;
    match value {
        Value::String(s) => Iri::parse(s)
            .map_err(|e| boundary_failure(format!("malformed IRI in `{prop_iri}`: {e}"))),
        Value::ResourceRef(iri) => Ok(iri.clone()),
        _ => Err(boundary_failure(format!(
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

/// Re-export so kernel-side implementations can see the property
/// IRIs without copying them. The boundary check owns these
/// constants because it's the place where their semantics matter.
pub mod props {
    pub const REQUIRES_ENVIRONMENT: &str = super::PROP_REQUIRES_ENVIRONMENT;
    pub const REQUIRES_MIRROR_CLASSES: &str = super::PROP_REQUIRES_MIRROR_CLASSES;
    pub const MIRROR_DEPENDENCY: &str = super::PROP_MIRROR_DEPENDENCY;
    pub const SOURCE_LAYER: &str = super::PROP_SOURCE_LAYER;
    pub const MIRRORED_CLASSES: &str = super::PROP_MIRRORED_CLASSES;
    pub const INPUT_TYPES: &str = super::PROP_INPUT_TYPES;
    pub const OUTPUT_TYPE: &str = super::PROP_OUTPUT_TYPE;
    pub const METHOD_PACKAGE: &str = super::PROP_METHOD_PACKAGE;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Synthetic in-memory chain for the boundary tests. Each layer
    /// is a `BTreeMap<Iri, Resource>` plus a parent pointer; a class
    /// defined at multiple layers takes the most-recent (closest to
    /// claim) definition; class-equality uses Resource's PartialEq
    /// (semantically equivalent to the canonical-CBOR-hash check the
    /// kernel adapter will use).
    struct FakeChain {
        layers: BTreeMap<Iri, FakeLayer>,
    }

    struct FakeLayer {
        parent: Option<Iri>,
        resources: BTreeMap<Iri, Resource>,
    }

    impl FakeChain {
        fn new() -> Self {
            Self {
                layers: BTreeMap::new(),
            }
        }

        fn add_layer(&mut self, id: &str, parent: Option<&str>) -> &mut FakeLayer {
            let iri = Iri::parse(id).unwrap();
            self.layers.insert(
                iri.clone(),
                FakeLayer {
                    parent: parent.map(|p| Iri::parse(p).unwrap()),
                    resources: BTreeMap::new(),
                },
            );
            self.layers.get_mut(&iri).unwrap()
        }

        fn ancestors_of(&self, claim: &Iri) -> Vec<Iri> {
            let mut out = vec![claim.clone()];
            let mut cur = claim.clone();
            while let Some(parent) = self.layers.get(&cur).and_then(|l| l.parent.clone()) {
                out.push(parent.clone());
                cur = parent;
            }
            out
        }
    }

    impl ChainAccessor for FakeChain {
        fn resolve(&self, claim_layer: &Iri, target: &Iri) -> Option<Resource> {
            for ancestor in self.ancestors_of(claim_layer) {
                let layer = self.layers.get(&ancestor)?;
                if let Some(r) = layer.resources.get(target) {
                    return Some(r.clone());
                }
            }
            None
        }

        fn is_ancestor_or_equal(&self, anchor: &Iri, candidate: &Iri) -> bool {
            self.ancestors_of(candidate).iter().any(|a| a == anchor)
        }

        fn class_unchanged_between(
            &self,
            mirror_layer: &Iri,
            claim_layer: &Iri,
            class_iri: &Iri,
        ) -> bool {
            // Only valid if mirror_layer is ancestral-or-equal.
            if !self.is_ancestor_or_equal(mirror_layer, claim_layer) {
                return false;
            }
            // Walk from claim down to (and including) mirror_layer; the
            // class is unchanged iff every layer on that path either
            // doesn't redefine the class or redefines it identically.
            let mirror_def = self
                .layers
                .get(mirror_layer)
                .and_then(|l| l.resources.get(class_iri));
            let mirror_def = match mirror_def {
                Some(d) => d,
                None => return false,
            };
            let mut cur = claim_layer.clone();
            loop {
                let layer = match self.layers.get(&cur) {
                    Some(l) => l,
                    None => return false,
                };
                if let Some(def) = layer.resources.get(class_iri) {
                    if def != mirror_def {
                        return false;
                    }
                }
                if &cur == mirror_layer {
                    return true;
                }
                cur = match layer.parent.clone() {
                    Some(p) => p,
                    None => return false,
                };
            }
        }
    }

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Build a class Resource with a single distinguishing property
    /// so we can test "modified" vs "unchanged".
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
            r.set(iri(props::MIRROR_DEPENDENCY), Value::ResourceRef(iri(m)));
        }
        r
    }

    fn mirror_resource(
        mirror_iri: &str,
        source_layer: &str,
        mirrored_classes: &[&str],
    ) -> Resource {
        let mut r = Resource::new(iri(mirror_iri));
        r.set(
            iri(props::SOURCE_LAYER),
            Value::String(source_layer.to_string()),
        );
        r.set(
            iri(props::MIRRORED_CLASSES),
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
            iri(props::REQUIRES_ENVIRONMENT),
            Value::ResourceRef(iri(env_iri)),
        );
        r.set(
            iri(props::REQUIRES_MIRROR_CLASSES),
            Value::Array(
                requires_classes
                    .iter()
                    .map(|c| Value::ResourceRef(iri(c)))
                    .collect(),
            ),
        );
        r
    }

    /// L0 ⊏ L1, mirror at L0 covers class C, script claims L1, no
    /// changes between layers → boundary check passes.
    #[test]
    fn passes_when_chain_compatible_and_classes_unchanged() {
        let mut chain = FakeChain::new();
        let l0 = chain.add_layer("urn:layer:l0", None);
        l0.resources.insert(
            iri("urn:eigenius:test:class:C"),
            class_resource("urn:eigenius:test:class:C", "v1"),
        );
        l0.resources.insert(
            iri("urn:eigenius:test:env:e1"),
            env_resource(
                "urn:eigenius:test:env:e1",
                Some("urn:eigenius:test:mirror:m1"),
            ),
        );
        l0.resources.insert(
            iri("urn:eigenius:test:mirror:m1"),
            mirror_resource(
                "urn:eigenius:test:mirror:m1",
                "urn:layer:l0",
                &["urn:eigenius:test:class:C"],
            ),
        );
        chain.add_layer("urn:layer:l1", Some("urn:layer:l0"));

        let script = script_resource("urn:eigenius:test:env:e1", &["urn:eigenius:test:class:C"]);
        check_run_script(&script, &iri("urn:layer:l1"), &chain).expect("should pass");
    }

    /// Mirror at L0, but L1 redefines class C → MirrorVersionMismatch
    /// pinpointing C.
    #[test]
    fn mirror_version_mismatch_when_class_redefined_in_descendant() {
        let mut chain = FakeChain::new();
        let l0 = chain.add_layer("urn:layer:l0", None);
        l0.resources.insert(
            iri("urn:eigenius:test:class:C"),
            class_resource("urn:eigenius:test:class:C", "v1"),
        );
        l0.resources.insert(
            iri("urn:eigenius:test:env:e1"),
            env_resource(
                "urn:eigenius:test:env:e1",
                Some("urn:eigenius:test:mirror:m1"),
            ),
        );
        l0.resources.insert(
            iri("urn:eigenius:test:mirror:m1"),
            mirror_resource(
                "urn:eigenius:test:mirror:m1",
                "urn:layer:l0",
                &["urn:eigenius:test:class:C"],
            ),
        );
        let l1 = chain.add_layer("urn:layer:l1", Some("urn:layer:l0"));
        // Redefine C with a new marker.
        l1.resources.insert(
            iri("urn:eigenius:test:class:C"),
            class_resource("urn:eigenius:test:class:C", "v2"),
        );

        let script = script_resource("urn:eigenius:test:env:e1", &["urn:eigenius:test:class:C"]);
        let err = check_run_script(&script, &iri("urn:layer:l1"), &chain)
            .expect_err("expected MirrorVersionMismatch");
        match err {
            RunError::MirrorVersionMismatch {
                class_iri,
                mirror_layer,
                claim_layer,
            } => {
                assert_eq!(class_iri, "urn:eigenius:test:class:C");
                assert_eq!(mirror_layer, "urn:layer:l0");
                assert_eq!(claim_layer, "urn:layer:l1");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Script's `requires_mirror_classes` includes a class not in the
    /// mirror's `mirrored_classes` → MissingMirrorStruct.
    #[test]
    fn missing_mirror_struct_when_required_class_not_mirrored() {
        let mut chain = FakeChain::new();
        let l0 = chain.add_layer("urn:layer:l0", None);
        l0.resources.insert(
            iri("urn:eigenius:test:class:A"),
            class_resource("urn:eigenius:test:class:A", "v1"),
        );
        // Note: B is referenced by the script but NOT in the mirror.
        l0.resources.insert(
            iri("urn:eigenius:test:env:e1"),
            env_resource(
                "urn:eigenius:test:env:e1",
                Some("urn:eigenius:test:mirror:m1"),
            ),
        );
        l0.resources.insert(
            iri("urn:eigenius:test:mirror:m1"),
            mirror_resource(
                "urn:eigenius:test:mirror:m1",
                "urn:layer:l0",
                &["urn:eigenius:test:class:A"],
            ),
        );

        let script = script_resource(
            "urn:eigenius:test:env:e1",
            &["urn:eigenius:test:class:A", "urn:eigenius:test:class:B"],
        );
        let err = check_run_script(&script, &iri("urn:layer:l0"), &chain)
            .expect_err("expected MissingMirrorStruct");
        match err {
            RunError::MissingMirrorStruct { class_iri } => {
                assert_eq!(class_iri, "urn:eigenius:test:class:B");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Mirror anchored to layer that's not on the claim's ancestor
    /// chain → MirrorAnchorNotAncestral.
    #[test]
    fn mirror_anchor_not_ancestral() {
        let mut chain = FakeChain::new();
        // L0 is the claim's chain root; LX is unrelated.
        chain.add_layer("urn:layer:l0", None);
        let l1 = chain.add_layer("urn:layer:l1", Some("urn:layer:l0"));
        l1.resources.insert(
            iri("urn:eigenius:test:class:C"),
            class_resource("urn:eigenius:test:class:C", "v1"),
        );
        l1.resources.insert(
            iri("urn:eigenius:test:env:e1"),
            env_resource(
                "urn:eigenius:test:env:e1",
                Some("urn:eigenius:test:mirror:m1"),
            ),
        );
        l1.resources.insert(
            iri("urn:eigenius:test:mirror:m1"),
            mirror_resource(
                "urn:eigenius:test:mirror:m1",
                "urn:layer:lx", // unrelated layer
                &["urn:eigenius:test:class:C"],
            ),
        );
        chain.add_layer("urn:layer:lx", None);

        let script = script_resource("urn:eigenius:test:env:e1", &["urn:eigenius:test:class:C"]);
        let err = check_run_script(&script, &iri("urn:layer:l1"), &chain)
            .expect_err("expected MirrorAnchorNotAncestral");
        match err {
            RunError::MirrorAnchorNotAncestral {
                mirror_layer,
                claim_layer,
            } => {
                assert_eq!(mirror_layer, "urn:layer:lx");
                assert_eq!(claim_layer, "urn:layer:l1");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Env without a `mirror_dependency` is permitted — the test
    /// runtime itself uses no mirror, so the boundary check has
    /// nothing to enforce. Returns Ok unconditionally.
    #[test]
    fn passes_when_env_has_no_mirror() {
        let mut chain = FakeChain::new();
        let l0 = chain.add_layer("urn:layer:l0", None);
        l0.resources.insert(
            iri("urn:eigenius:test:env:e1"),
            env_resource("urn:eigenius:test:env:e1", None),
        );

        let script = script_resource(
            "urn:eigenius:test:env:e1",
            &["urn:eigenius:test:class:NeverMirrored"],
        );
        check_run_script(&script, &iri("urn:layer:l0"), &chain).expect("should pass");
    }

    /// Method check covers `input_types` + `output_type`. A method
    /// whose output_type isn't mirrored fails the same way a script
    /// referencing an unmirrored class does.
    #[test]
    fn check_call_method_covers_input_and_output_types() {
        let mut chain = FakeChain::new();
        let l0 = chain.add_layer("urn:layer:l0", None);
        l0.resources.insert(
            iri("urn:eigenius:test:class:Input"),
            class_resource("urn:eigenius:test:class:Input", "v1"),
        );
        l0.resources.insert(
            iri("urn:eigenius:test:env:e1"),
            env_resource(
                "urn:eigenius:test:env:e1",
                Some("urn:eigenius:test:mirror:m1"),
            ),
        );
        l0.resources.insert(
            iri("urn:eigenius:test:mirror:m1"),
            mirror_resource(
                "urn:eigenius:test:mirror:m1",
                "urn:layer:l0",
                &["urn:eigenius:test:class:Input"], // Output not mirrored
            ),
        );

        let mut signature = Resource::new(iri("urn:eigenius:test:sig:s1"));
        signature.set(
            iri(props::INPUT_TYPES),
            Value::Array(vec![Value::ResourceRef(iri(
                "urn:eigenius:test:class:Input",
            ))]),
        );
        signature.set(
            iri(props::OUTPUT_TYPE),
            Value::ResourceRef(iri("urn:eigenius:test:class:Output")),
        );

        let err = check_call_method(
            &signature,
            &iri("urn:eigenius:test:env:e1"),
            &iri("urn:layer:l0"),
            &chain,
        )
        .expect_err("expected MissingMirrorStruct on output type");
        match err {
            RunError::MissingMirrorStruct { class_iri } => {
                assert_eq!(class_iri, "urn:eigenius:test:class:Output");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Sanity check on the test fixture's distinct-class semantics —
    /// a class that's been "redefined identically" on a descendant
    /// layer should still pass (defensive against false positives).
    #[test]
    fn passes_when_class_redefined_identically() {
        let mut chain = FakeChain::new();
        let l0 = chain.add_layer("urn:layer:l0", None);
        l0.resources.insert(
            iri("urn:eigenius:test:class:C"),
            class_resource("urn:eigenius:test:class:C", "v1"),
        );
        l0.resources.insert(
            iri("urn:eigenius:test:env:e1"),
            env_resource(
                "urn:eigenius:test:env:e1",
                Some("urn:eigenius:test:mirror:m1"),
            ),
        );
        l0.resources.insert(
            iri("urn:eigenius:test:mirror:m1"),
            mirror_resource(
                "urn:eigenius:test:mirror:m1",
                "urn:layer:l0",
                &["urn:eigenius:test:class:C"],
            ),
        );
        let l1 = chain.add_layer("urn:layer:l1", Some("urn:layer:l0"));
        // Redefine C with the SAME marker — should be considered
        // unchanged.
        l1.resources.insert(
            iri("urn:eigenius:test:class:C"),
            class_resource("urn:eigenius:test:class:C", "v1"),
        );

        let script = script_resource("urn:eigenius:test:env:e1", &["urn:eigenius:test:class:C"]);
        check_run_script(&script, &iri("urn:layer:l1"), &chain).expect("should pass");
    }
}
