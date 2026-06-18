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

//! Eigenius Runtime Substrate
//!
//! Language-agnostic substrate for hosting external language toolchains
//! (Julia, Python, R, Lean's authoring side, …) inside Eigenius with full
//! provenance. Consumers implement [`LanguageRuntime`] to plug a
//! concrete language in; the substrate provides the trait, the worker RPC
//! framing, the boundary check, the image-vs-graph split, and the
//! `RunRuntimeScript` / `CallRuntimeMethod` substrate components.
//!
//! The parent ontology classes (`RuntimeScript`, `RuntimePackage`,
//! `RuntimeEnvironment`, `RuntimePackageMirror`, `RuntimeInvocation`,
//! `RuntimePackagePin`, `RuntimeMethodSignature`, plus the `DispatchedTo`
//! morphism class) are committed as Eigon resources by the kernel
//! bootstrap from
//! `ontologies/runtime/runtime-substrate-ontology.json`. Per-language
//! crates commit subclasses of these in their own ontologies.
//!
//! See [D26 Runtime Substrate](../../../docs/design/d26-runtime-substrate.md)
//! for the full specification and Phase 18 of the implementation plan
//! for the milestones this crate maps onto.

pub mod boundary;
pub mod chain;
pub mod content_address;
pub mod cross_check;
pub mod dataset_schema;
pub mod error;
pub mod external_file;
pub mod facade;
pub mod image_build;
pub mod invocation;
pub mod language_runtime;
pub mod mirror_generator;
pub mod oxen;
pub mod registry;
pub mod rpc;
pub mod spawner;
#[cfg(feature = "test-runtime")]
pub mod test_runtime;
#[cfg(all(feature = "test-runtime", feature = "docker-spawner"))]
pub mod test_runtime_docker;
// `test_runtime_julia` removed in Phase 19a.1 — the production
// implementation moved to the `eigenius-julia` crate. See
// `eigenius_julia::JuliaLanguageRuntime`.
pub mod types;

pub use boundary::{check_call_method, check_run_script};
pub use chain::ChainAccessor;
pub use content_address::{
    content_hash_of, content_hash_of_file, pinned_external_file_iri, ContentAddressError,
    RuntimeScriptIdentity, PINNED_EXTERNAL_FILE_IRI_PREFIX, RUNTIME_SCRIPT_IRI_PREFIX,
};
pub use cross_check::{
    is_cross_check_failure, prepare_substrate_side, verify_in_worker, CrossCheckError,
    CrossCheckOutcome, ProvenanceDirAction, SubstratePrepareError, DEFAULT_PROVENANCE_DIR,
    ENV_DIGEST_VAR, ENV_MANIFEST_HASH_VAR, ENV_PROVENANCE_DIR_VAR, EXIT_CODE_CROSS_CHECK_FAILURE,
    MANIFEST_HASH_FILE,
};
pub use dataset_schema::{
    header_columns, parse_dataset_schema, validate_collection, validate_tabular, Attribute,
    DatasetSchema, Dimension, Layout, LayoutKind, Measure,
};
pub use error::{BuildError, ResourceLimit, RunError, SpawnError};
pub use external_file::{
    is_pinned_external_file, prepare_input, resolve_and_materialize, ResolveOptions,
    PROP_MATERIALIZED_PATH,
};
pub use facade::{FacadeError, SubstrateDispatcher};
pub use image_build::{
    compose_dockerfile, is_buildah_available, BuildContext, BuildContextSpec, BuildahImageBuilder,
    DockerfileSpec, ImageBuilder, IncludedPackage, LanguageAsset, MirrorMaterialization,
    PackageMaterialization,
};
pub use invocation::{numerical_metadata_to_json, DispatchTrace, RunOutcome};
pub use language_runtime::LanguageRuntime;
pub use mirror_generator::{
    LibraryContent, LibraryFile, MirrorGenerationOutput, MirrorGenerationRequest, MirrorGenerator,
    MirrorGeneratorError, MirrorGeneratorRegistry, MirrorRegistryError,
};
pub use registry::{LanguageRuntimeRegistry, RegistryError};
pub use rpc::{
    ClientError, FrameError, HealthInfo, NumericalMetadata, Request, Response, WorkerRpcClient,
    MAX_FRAME_SIZE_DEFAULT,
};
#[cfg(feature = "docker-spawner")]
pub use spawner::{
    DockerServiceSpawner, DockerSpawner, DockerSpawnerConfig, NetworkMode, PullPolicy,
};
pub use spawner::{
    LocalServiceSpawner, LocalSpawner, ServiceHandle, ServiceSpawner, WorkerSpawner,
};
#[cfg(feature = "test-runtime")]
pub use test_runtime::TestLanguageRuntime;
#[cfg(all(feature = "test-runtime", feature = "docker-spawner"))]
pub use test_runtime_docker::TestLanguageRuntimeDocker;
// `TestLanguageRuntimeJulia` re-export removed in Phase 19a.1 —
// production type is `eigenius_julia::JuliaLanguageRuntime`.
pub use types::{DockerfileFragments, ImageDigest, ImageDigestError, WorkerHandle, WorkerSpec};
