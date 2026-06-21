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

//! The kernel-tracked `runtime:BuildRecipe` (D60 §4.2): a chain-resident,
//! content-verified record of how a `RuntimeEnvironment` image was built — base
//! image, the composed Dockerfile, the content hashes of each baked artifact, the
//! exact build command, and the builder + version. `eigenius env build` commits it
//! alongside the environment (via `runtime:build_recipe`); `--verify` reproduces
//! the image digest from it and fails closed on drift.
//!
//! The builder here is generic (any runtime can emit a recipe); `OciToolRuntime`
//! is the first caller.

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use sha2::{Digest, Sha256};

pub const BUILD_RECIPE_CLASS: &str = "urn:eigenius:runtime:BuildRecipe";
const IS_A: &str = "urn:eigenius:core:is_a";
const SHORT_NAME: &str = "urn:eigenius:core:short_name";
const BASE_IMAGE: &str = "urn:eigenius:runtime:base_image";
const DOCKERFILE: &str = "urn:eigenius:runtime:dockerfile";
const BUILD_COMMAND: &str = "urn:eigenius:runtime:build_command";
const BUILDER: &str = "urn:eigenius:runtime:builder";
const BUILDER_VERSION: &str = "urn:eigenius:runtime:builder_version";
const ARTIFACT_HASHES: &str = "urn:eigenius:runtime:artifact_hashes";

/// The inputs to a deterministic image build, recorded as a `BuildRecipe`.
pub struct RecipeInputs<'a> {
    /// Digest-pinned base image the build extends.
    pub base_image: &'a str,
    /// The verbatim composed Dockerfile the builder ran.
    pub dockerfile: &'a str,
    /// The exact `eigenius env build …` invocation (argv joined).
    pub build_command: &'a str,
    /// The image builder (e.g. `buildah`).
    pub builder: &'a str,
    /// The builder's version string (e.g. `buildah 1.33.0`).
    pub builder_version: &'a str,
    /// `name:sha256:<hex>` for each baked-in artifact (e.g. the worker binary).
    pub artifact_hashes: &'a [String],
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-known IRI")
}

/// Content-addressed IRI for a recipe — deterministic in its inputs, so the same
/// build recipe always converges to the same node.
fn recipe_iri(inputs: &RecipeInputs) -> String {
    let mut h = Sha256::new();
    for field in [
        inputs.base_image,
        inputs.dockerfile,
        inputs.build_command,
        inputs.builder,
        inputs.builder_version,
    ] {
        h.update(field.as_bytes());
        h.update(b"\0");
    }
    for a in inputs.artifact_hashes {
        h.update(a.as_bytes());
        h.update(b"\0");
    }
    format!("urn:eigenius:runtime:recipe:{:x}", h.finalize())
}

/// Build the `BuildRecipe` `Resource` (content-addressed) from the inputs.
pub fn build_recipe_resource(inputs: &RecipeInputs) -> Resource {
    let mut r = Resource::new(iri(&recipe_iri(inputs)));
    r.set(
        iri(IS_A),
        Value::Array(vec![Value::ResourceRef(iri(BUILD_RECIPE_CLASS))]),
    );
    r.set(iri(SHORT_NAME), Value::String("build_recipe".to_string()));
    r.set(
        iri(BASE_IMAGE),
        Value::String(inputs.base_image.to_string()),
    );
    r.set(
        iri(DOCKERFILE),
        Value::String(inputs.dockerfile.to_string()),
    );
    r.set(
        iri(BUILD_COMMAND),
        Value::String(inputs.build_command.to_string()),
    );
    r.set(iri(BUILDER), Value::String(inputs.builder.to_string()));
    r.set(
        iri(BUILDER_VERSION),
        Value::String(inputs.builder_version.to_string()),
    );
    r.set(
        iri(ARTIFACT_HASHES),
        Value::Array(
            inputs
                .artifact_hashes
                .iter()
                .map(|a| Value::String(a.clone()))
                .collect(),
        ),
    );
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
    use eigenius_kernel::ontology::eigon_json;
    use eigenius_kernel::validation::Validator;
    use std::sync::Arc;

    fn sample() -> RecipeInputs<'static> {
        RecipeInputs {
            base_image: "debian:bookworm-slim@sha256:abc",
            dockerfile: "FROM debian:bookworm-slim\nCOPY worker /usr/local/bin/\n",
            build_command: "eigenius env build --language oci --worker eigenius-schemaorg-worker",
            builder: "buildah",
            builder_version: "buildah 1.33.0",
            artifact_hashes: &[],
        }
    }

    #[test]
    fn recipe_carries_required_fields() {
        let hashes = vec!["eigenius-oci-worker:sha256:deadbeef".to_string()];
        let inputs = RecipeInputs {
            artifact_hashes: &hashes,
            ..sample()
        };
        let r = build_recipe_resource(&inputs);
        assert!(r.is_a().iter().any(|c| c.as_str() == BUILD_RECIPE_CLASS));
        assert_eq!(
            r.get(&iri(BASE_IMAGE)).and_then(Value::as_str),
            Some("debian:bookworm-slim@sha256:abc"),
        );
        assert_eq!(
            r.get(&iri(BUILDER)).and_then(Value::as_str),
            Some("buildah")
        );
        match r.get(&iri(ARTIFACT_HASHES)) {
            Some(Value::Array(a)) => assert_eq!(a.len(), 1),
            other => panic!("artifact_hashes should be an array, got {other:?}"),
        }
    }

    #[test]
    fn recipe_iri_is_deterministic_in_inputs() {
        let a = build_recipe_resource(&sample());
        let b = build_recipe_resource(&sample());
        assert_eq!(a.id(), b.id());
        // A different base image yields a different node.
        let c = build_recipe_resource(&RecipeInputs {
            base_image: "debian:bookworm-slim@sha256:zzz",
            ..sample()
        });
        assert_ne!(a.id(), c.id());
    }

    #[test]
    fn recipe_validates_against_the_runtime_ontology() {
        // core → runtime, then validate the recipe instance against the chain.
        let mut cb = LayerBuilder::new("core", None);
        for r in
            eigon_json::parse_document(include_str!("../../../ontologies/core/core-ontology.json"))
                .unwrap()
        {
            cb.add_resource(r).unwrap();
        }
        let core = Arc::new(cb.build(LayerStorage::in_memory()));

        let mut rb = LayerBuilder::new("runtime", Some(core));
        for r in eigon_json::parse_document(include_str!(
            "../../../ontologies/runtime/runtime-substrate-ontology.json"
        ))
        .unwrap()
        {
            rb.add_resource(r).unwrap();
        }
        let runtime = Arc::new(rb.build(LayerStorage::in_memory()));

        let hashes = vec!["eigenius-oci-worker:sha256:deadbeef".to_string()];
        let recipe = build_recipe_resource(&RecipeInputs {
            artifact_hashes: &hashes,
            ..sample()
        });
        let mut pb = LayerBuilder::new("recipe", Some(runtime));
        let recipe_id = recipe.id().unwrap().clone();
        pb.add_resource(recipe).unwrap();
        let layer = Arc::new(pb.build(LayerStorage::in_memory()));

        let errors = Validator::new(layer.clone())
            .validate_resource(&layer.resolve(&recipe_id).expect("recipe present"));
        assert!(
            errors.is_empty(),
            "the BuildRecipe instance must validate against runtime:BuildRecipe; errors: {errors:?}",
        );
    }
}
