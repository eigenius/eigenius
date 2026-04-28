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

//! Bootstrap sequence for kernel initialization.
//!
//! Loads the core ontology from `ontologies/core/core-ontology.json`,
//! creates the root layer, validates it against itself, and returns
//! a working execution context.

use crate::context::{ExecutionContext, ExecutionMode};
use crate::layer::{Layer, LayerBuilder};
use crate::observability::{field, operation};
use crate::ontology::eigon_json;
use crate::validation::Validator;
use std::fmt;
use std::sync::Arc;

/// Errors during bootstrap.
#[derive(Debug)]
pub enum BootstrapError {
    Parse(eigon_json::ParseError),
    Layer(crate::layer::LayerError),
    CoreOntologyInvalid(Vec<crate::validation::ValidationError>),
    Storage(String),
    ManifestDrift { stored: String, current: String },
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootstrapError::Parse(e) => write!(f, "failed to parse core ontology: {e}"),
            BootstrapError::Layer(e) => write!(f, "failed to build core layer: {e}"),
            BootstrapError::CoreOntologyInvalid(errors) => {
                writeln!(
                    f,
                    "core ontology validation failed with {} error(s):",
                    errors.len()
                )?;
                for e in errors {
                    writeln!(f, "  {e}")?;
                }
                Ok(())
            }
            BootstrapError::Storage(msg) => write!(f, "persistent backend error: {msg}"),
            BootstrapError::ManifestDrift { stored, current } => writeln!(
                f,
                "seed manifest drift — refusing to boot against a DB seeded with different embedded ontologies.\n\
                 stored:\n{stored}current:\n{current}\n\
                 Options:\n  1. Use a fresh --db path (reinstalls your capabilities).\n  \
                 2. Run a migration when `eigenius db migrate` lands (tracked as Phase 14)."
            ),
        }
    }
}

impl std::error::Error for BootstrapError {}

/// Load, build, and validate a layer from embedded JSON.
fn load_layer(
    name: &str,
    json: &str,
    parent: Option<Arc<Layer>>,
    cache: Arc<dyn crate::layer::ResourceCache>,
    backend: Arc<dyn crate::storage::ResourceBackend>,
) -> Result<Arc<Layer>, BootstrapError> {
    let resources = eigon_json::parse_document(json).map_err(BootstrapError::Parse)?;

    let resource_count = resources.len();
    let mut builder = LayerBuilder::new(name, parent);
    for resource in resources {
        builder
            .add_resource(resource)
            .map_err(BootstrapError::Layer)?;
    }
    let layer = Arc::new(builder.build(cache, backend));

    let validator = Validator::new(&layer);
    let errors = validator.validate();
    if !errors.is_empty() {
        tracing::warn!(
            { field::OPERATION } = operation::BOOTSTRAP_LOAD,
            layer_name = %name,
            { field::COUNT } = errors.len(),
            "embedded ontology validation produced errors"
        );
        return Err(BootstrapError::CoreOntologyInvalid(errors));
    }

    tracing::debug!(
        { field::OPERATION } = operation::BOOTSTRAP_LOAD,
        layer_name = %name,
        { field::LAYER_ID } = %layer.id(),
        { field::COUNT } = resource_count,
        "bootstrap layer loaded"
    );

    Ok(layer)
}

/// Bootstrap the Eigenius kernel.
///
/// Loads five ontology layers: core → program → reflection → institution → notebook.
/// All are validated. Returns an `ExecutionContext` with the
/// notebook layer as head.
///
/// Phase 14a-iii: an in-memory cache + backend are created here and shared
/// across all bootstrap layers and the returned `ExecutionContext`. Persistent
/// backends (`bootstrap_persistent`) replace the in-memory backend with a
/// `RocksStore` adapter.
pub fn bootstrap() -> Result<ExecutionContext, BootstrapError> {
    let cache: Arc<dyn crate::layer::ResourceCache> =
        Arc::new(crate::layer::MemoryResourceCache::new());
    let backend: Arc<dyn crate::storage::ResourceBackend> =
        Arc::new(crate::layer::MemoryResourceBackend::new());
    bootstrap_with_storage(cache, backend)
}

/// Bootstrap with caller-provided cache + backend. Used by
/// `bootstrap_persistent` (RocksDB-backed) and tests that need a particular
/// storage configuration.
pub fn bootstrap_with_storage(
    cache: Arc<dyn crate::layer::ResourceCache>,
    backend: Arc<dyn crate::storage::ResourceBackend>,
) -> Result<ExecutionContext, BootstrapError> {
    let core = load_layer(
        "core",
        include_str!("../../../ontologies/core/core-ontology.json"),
        None,
        Arc::clone(&cache),
        Arc::clone(&backend),
    )?;

    let program = load_layer(
        "program",
        include_str!("../../../ontologies/program/program-ontology.json"),
        Some(core),
        Arc::clone(&cache),
        Arc::clone(&backend),
    )?;

    let reflection = load_layer(
        "reflection",
        include_str!("../../../ontologies/reflection/reflection-ontology.json"),
        Some(program),
        Arc::clone(&cache),
        Arc::clone(&backend),
    )?;

    let institution = load_layer(
        "institution",
        include_str!("../../../ontologies/institution/institution-ontology.json"),
        Some(reflection),
        Arc::clone(&cache),
        Arc::clone(&backend),
    )?;

    let notebook = load_layer(
        "notebook",
        include_str!("../../../ontologies/notebook/notebook-ontology.json"),
        Some(institution),
        Arc::clone(&cache),
        Arc::clone(&backend),
    )?;

    Ok(ExecutionContext::new(
        notebook,
        "working",
        ExecutionMode::ReadWrite,
        cache,
        backend,
    ))
}

/// Bootstrap against a persistent backend (D13 §4).
///
/// Two paths:
///
/// - **SEED** (empty backend): run the normal in-memory bootstrap,
///   then commit each of the five embedded ontology layers
///   (core → program → reflection → institution → notebook) to the
///   backend in parent→child order, record the seed manifest, and
///   point the head at the notebook layer.
/// - **RESUME** (backend has a head): verify the stored seed manifest
///   matches the current embedded ontologies' SHA-256 hashes; if it
///   does, rehydrate the layer chain from the backend. If it doesn't,
///   return a `ManifestDrift` error with actionable detail.
pub fn bootstrap_persistent(
    backend: &dyn crate::storage::PersistentBackend,
) -> Result<ExecutionContext, BootstrapError> {
    match backend
        .get_head()
        .map_err(|e| BootstrapError::Storage(format!("get_head: {e}")))?
    {
        Some(_) => resume_from_backend(backend),
        None => seed_backend(backend),
    }
}

// --- SEED / RESUME helpers ---

const SEED_MANIFEST_KEY: &str = "seed_manifest_v1";

fn embedded_ontologies() -> [(&'static str, &'static str); 5] {
    [
        (
            "core",
            include_str!("../../../ontologies/core/core-ontology.json"),
        ),
        (
            "program",
            include_str!("../../../ontologies/program/program-ontology.json"),
        ),
        (
            "reflection",
            include_str!("../../../ontologies/reflection/reflection-ontology.json"),
        ),
        (
            "institution",
            include_str!("../../../ontologies/institution/institution-ontology.json"),
        ),
        (
            "notebook",
            include_str!("../../../ontologies/notebook/notebook-ontology.json"),
        ),
    ]
}

fn current_manifest() -> Vec<u8> {
    // Newline-separated "<name>:<sha256_hex>" lines, stable ordering.
    let mut out = String::new();
    for (name, json) in embedded_ontologies() {
        use sha2::Digest;
        let hash = sha2::Sha256::digest(json.as_bytes());
        out.push_str(&format!("{name}:{}\n", hex::encode(hash)));
    }
    out.into_bytes()
}

fn seed_backend(
    backend: &dyn crate::storage::PersistentBackend,
) -> Result<ExecutionContext, BootstrapError> {
    // Build the four ontologies in memory (reusing the existing path)
    // so they're validated before anything touches the DB.
    let ctx = bootstrap()?;

    // Walk the chain from the root (core) up and persist each layer.
    // Layer chain is head → parent → ... → core, so collect bottom-up.
    let mut chain: Vec<Arc<Layer>> = Vec::new();
    let mut cursor = Some(Arc::clone(ctx.head()));
    while let Some(layer) = cursor {
        let parent = layer.parent().cloned();
        chain.push(layer);
        cursor = parent;
    }
    chain.reverse(); // root (core) first

    for layer in &chain {
        backend
            .store_layer(layer)
            .map_err(|e| BootstrapError::Storage(format!("store_layer {}: {e}", layer.name())))?;
    }
    backend
        .set_head(ctx.head().id())
        .map_err(|e| BootstrapError::Storage(format!("set_head: {e}")))?;

    backend
        .put_meta(SEED_MANIFEST_KEY, &current_manifest())
        .map_err(|e| BootstrapError::Storage(format!("put_meta(manifest): {e}")))?;

    Ok(ctx)
}

fn resume_from_backend(
    backend: &dyn crate::storage::PersistentBackend,
) -> Result<ExecutionContext, BootstrapError> {
    // Manifest check before trusting the chain. If drift is detected we
    // refuse to boot rather than silently upgrading the ontology in
    // place (D13 §8).
    let stored = backend
        .get_meta(SEED_MANIFEST_KEY)
        .map_err(|e| BootstrapError::Storage(format!("get_meta(manifest): {e}")))?;
    let current = current_manifest();
    match stored {
        Some(bytes) if bytes == current => {}
        Some(bytes) => {
            return Err(BootstrapError::ManifestDrift {
                stored: String::from_utf8_lossy(&bytes).to_string(),
                current: String::from_utf8_lossy(&current).to_string(),
            });
        }
        None => {
            // Missing manifest on a non-empty DB: treat as drift rather
            // than silently trusting. Users with pre-9a DBs will need a
            // fresh path (v1 policy per D13 §8).
            return Err(BootstrapError::ManifestDrift {
                stored: "(missing)".to_string(),
                current: String::from_utf8_lossy(&current).to_string(),
            });
        }
    }

    let info = backend
        .load_chain()
        .map_err(|e| BootstrapError::Storage(format!("load_chain: {e}")))?
        .ok_or_else(|| {
            BootstrapError::Storage("head pointer set but chain load returned None".into())
        })?;

    let cache: Arc<dyn crate::layer::ResourceCache> =
        Arc::new(crate::layer::MemoryResourceCache::new());
    // The persistent backend Arc must come from the caller for proper Arc-
    // sharing; bootstrap_persistent currently takes `&dyn`. Construct a
    // throw-away in-memory backend to satisfy the type and warm the cache
    // ourselves below — reads through the rebuilt chain hit the in-memory
    // backend (cache-only). This is a known-suboptimal interim choice; the
    // server-side caller should switch to `Arc<dyn PersistentBackend>` so
    // the chain references the real RocksDB backend (follow-up).
    let resource_backend: Arc<dyn crate::storage::ResourceBackend> =
        Arc::new(crate::layer::MemoryResourceBackend::new());

    // Warm cache from the persistent backend so cache-only reads work.
    for handle in &info.handles {
        if let Some(iris) = info.defined_iris_per_layer.get(&handle.id) {
            for iri in iris {
                if let Some(resource) = backend.load_resource(&handle.id, iri) {
                    cache.put(
                        crate::layer::ResourceKey::new(handle.id.clone(), iri.clone()),
                        Arc::new(resource),
                    );
                }
            }
        }
    }

    let head = crate::layer::build_chain(info, Arc::clone(&cache), Arc::clone(&resource_backend));

    Ok(ExecutionContext::new(
        head,
        "working",
        ExecutionMode::ReadWrite,
        cache,
        resource_backend,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology::iri::Iri;

    #[test]
    fn bootstrap_succeeds() {
        let ctx = bootstrap().unwrap();
        // Head is the notebook layer
        // (on top of institution → reflection → program → core)
        assert!(!ctx.head().is_root());
        let institution = ctx.head().parent().unwrap();
        assert!(!institution.is_root());
        let reflection = institution.parent().unwrap();
        assert!(!reflection.is_root());
        let program = reflection.parent().unwrap();
        assert!(!program.is_root());
        // Core layer (parent of program) should be root
        assert!(program.parent().unwrap().is_root());
    }

    #[test]
    fn can_resolve_core_resources() {
        let ctx = bootstrap().unwrap();
        let class_iri = Iri::parse("urn:eigenius:core:Class").unwrap();
        let resolved = ctx.resolve(&class_iri);
        assert!(
            resolved.is_some(),
            "should resolve Class from core ontology"
        );
    }

    #[test]
    fn can_resolve_all_core_classes() {
        let ctx = bootstrap().unwrap();
        for class_name in [
            "Class",
            "Property",
            "DataType",
            "Format",
            "Encoding",
            "ConditionalRequirement",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:core:{class_name}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve core class {class_name}"
            );
        }
    }

    #[test]
    fn can_resolve_core_properties() {
        let ctx = bootstrap().unwrap();
        for prop in [
            "is_a",
            "description",
            "short_name",
            "data_type",
            "requires",
            "recommends",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:core:{prop}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve core property {prop}"
            );
        }
    }

    #[test]
    fn can_resolve_data_types() {
        let ctx = bootstrap().unwrap();
        for dt in [
            "string",
            "integer",
            "float",
            "boolean",
            "resource",
            "resource_array",
            "value_array",
            "json",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:core:{dt}")).unwrap();
            assert!(ctx.resolve(&iri).is_some(), "should resolve data type {dt}");
        }
    }

    #[test]
    fn can_resolve_formats() {
        let ctx = bootstrap().unwrap();
        for fmt in ["date", "datetime", "time", "iri", "uuid", "regex"] {
            let iri = Iri::parse(&format!("urn:eigenius:core:formats:{fmt}")).unwrap();
            assert!(ctx.resolve(&iri).is_some(), "should resolve format {fmt}");
        }
    }

    #[test]
    fn can_resolve_program_classes() {
        let ctx = bootstrap().unwrap();
        for class in [
            "Program",
            "Let",
            "Apply",
            "Var",
            "Lambda",
            "Case",
            "Branch",
            "Pair",
            "Construct",
            "Project",
            "Map",
            "Reduce",
            "Literal",
            "Component",
            "CapabilityLevel",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:program:{class}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve program class {class}"
            );
        }
    }

    #[test]
    fn can_resolve_builtin_components() {
        let ctx = bootstrap().unwrap();
        for comp in [
            "Identity",
            "CompleteText",
            "CompleteJson",
            "Combine",
            "Extract",
            "Transform",
            "HttpRequest",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:program:components:{comp}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve component {comp}"
            );
        }
    }

    #[test]
    fn can_resolve_reflection_classes() {
        let ctx = bootstrap().unwrap();
        for class in [
            "DeclaredResource",
            "ObservedResource",
            "DerivedResource",
            "VerifiedResource",
            "ComponentTrace",
            "ProgramTrace",
            "DeclarationTrace",
            "ObservationTrace",
            "VerificationTrace",
            "LetTrace",
            "MapTrace",
            "CaseTrace",
            "ConstructTrace",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:reflection:{class}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve reflection class {class}"
            );
        }
    }

    #[test]
    fn can_resolve_epistemic_statuses() {
        let ctx = bootstrap().unwrap();
        for status in ["declared", "observed", "derived", "verified"] {
            let iri = Iri::parse(&format!("urn:eigenius:reflection:epistemic:{status}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve epistemic status {status}"
            );
        }
    }

    #[test]
    fn can_resolve_institution_classes() {
        let ctx = bootstrap().unwrap();
        for class in [
            "Institution",
            "FiberMorphism",
            "FiberQuery",
            "StructuralProperty",
            "PropertyKind",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:institution:{class}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve institution class {class}"
            );
        }
    }

    #[test]
    fn can_resolve_property_kinds() {
        let ctx = bootstrap().unwrap();
        for kind in ["reflexive", "transitive", "symmetric", "antisymmetric"] {
            let iri =
                Iri::parse(&format!("urn:eigenius:institution:property_kinds:{kind}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve property kind {kind}"
            );
        }
    }
}
