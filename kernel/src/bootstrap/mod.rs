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
    ManifestDrift {
        stored: String,
        current: String,
    },
    /// D24 §2: non-empty DB without `meta:schema_version`. The DB
    /// was written by a pre-marker kernel (Phase 14 or earlier);
    /// the operator must re-seed against a fresh `--db` path.
    SchemaVersionAbsent,
    /// D24 §2: stored schema version is higher than this kernel's
    /// `SCHEMA_VERSION`. Older kernel against a newer DB.
    SchemaTooNew {
        stored: u32,
        kernel: u32,
    },
    /// D24 §2: stored schema version is lower than this kernel's
    /// `SCHEMA_VERSION` and no contiguous migration chain exists in
    /// the kernel's `MigrationRegistry` to bridge the gap.
    NoMigrationPath {
        from: u32,
        to: u32,
    },
    /// D24 §2: a registered migration ran but returned an error. The
    /// DB is left at its pre-migration version (migrations are
    /// required to be atomic).
    MigrationFailed {
        from: u32,
        to: u32,
        message: String,
    },
    /// D24 §3.1: the on-disk `meta:schema_version` value couldn't be
    /// decoded. Indicates corruption rather than version mismatch —
    /// no migration path applies.
    SchemaVersionCorrupt(String),
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
            BootstrapError::SchemaVersionAbsent => write!(
                f,
                "schema version marker absent — refusing to boot against a non-empty DB without \
                 `meta:schema_version`.\n\
                 The DB was written by a pre-marker kernel (Phase 14 or earlier). Re-seed against \
                 a fresh `--db` path; pre-marker DBs are not supported.\n\
                 See docs/design/d24-schema-versioning.md."
            ),
            BootstrapError::SchemaTooNew { stored, kernel } => write!(
                f,
                "schema version too new — DB stamped at v{stored}, this kernel expects v{kernel}.\n\
                 Upgrade the kernel binary; older kernels cannot open DBs migrated past their \
                 expected version (D24 §5: no forward compatibility)."
            ),
            BootstrapError::NoMigrationPath { from, to } => write!(
                f,
                "no migration path from schema v{from} to v{to}. The kernel's `MigrationRegistry` \
                 lacks a contiguous chain.\n\
                 This usually means the kernel was built with `SCHEMA_VERSION={to}` but the \
                 corresponding `vN_to_vN+1` migrations were not all registered. Report this as a \
                 kernel bug; see docs/design/d24-schema-versioning.md §6.1."
            ),
            BootstrapError::MigrationFailed { from, to, message } => write!(
                f,
                "migration v{from} → v{to} failed: {message}\n\
                 The DB is left at its pre-migration version. Inspect the error above and \
                 retry; migrations are required to be atomic and idempotent (D24 §3.3)."
            ),
            BootstrapError::SchemaVersionCorrupt(detail) => write!(
                f,
                "`meta:schema_version` is corrupt: {detail}\n\
                 The version marker exists but cannot be decoded. This indicates DB corruption, \
                 not a version mismatch — no migration applies. Restore from backup."
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
    storage: crate::layer::LayerStorage,
) -> Result<Arc<Layer>, BootstrapError> {
    let resources = eigon_json::parse_document(json).map_err(BootstrapError::Parse)?;
    build_layer_from_resources(name, resources, parent, storage)
}

/// ESL-sourced variant of [`load_layer`]. Compiles the ESL source into
/// chain resources via `esl::compile`, then runs the same
/// build-and-validate pipeline. Used by Phase-8 bootstrap to ship
/// `ontologies/reasoning/reasoning.esl` without committing a parallel
/// `.json` to keep in sync — single source of truth.
fn load_esl_layer(
    name: &str,
    esl_source: &str,
    parent: Option<Arc<Layer>>,
    storage: crate::layer::LayerStorage,
) -> Result<Arc<Layer>, BootstrapError> {
    let resources = crate::esl::compile(esl_source).map_err(|errs| {
        BootstrapError::CoreOntologyInvalid(
            errs.into_iter()
                .map(|e| crate::validation::ValidationError {
                    resource_id: None,
                    property: None,
                    rule: crate::validation::ValidationRule::TypeMismatch,
                    message: format!("ESL compile error in bootstrap layer `{name}`: {e:?}"),
                })
                .collect(),
        )
    })?;
    build_layer_from_resources(name, resources, parent, storage)
}

fn build_layer_from_resources(
    name: &str,
    resources: Vec<crate::ontology::resource::Resource>,
    parent: Option<Arc<Layer>>,
    storage: crate::layer::LayerStorage,
) -> Result<Arc<Layer>, BootstrapError> {
    let resource_count = resources.len();
    let mut builder = LayerBuilder::new(name, parent);
    for resource in resources {
        builder
            .add_resource(resource)
            .map_err(BootstrapError::Layer)?;
    }
    let layer = Arc::new(builder.build(storage));

    let validator = Validator::new(Arc::clone(&layer));
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
/// Loads thirteen ontology layers: core → eigentt-type-fragment → program → reflection → obo → institution → runtime → formulas → lean-expressions → lean-runtime-classes → lean-institution → reasoning → notebook.
/// All are validated. Returns an `ExecutionContext` with the
/// notebook layer as head.
///
/// Phase 14a-iii: an in-memory cache + backend are created here and shared
/// across all bootstrap layers and the returned `ExecutionContext`. Persistent
/// backends (`bootstrap_persistent`) replace the in-memory backend with a
/// `RocksStore` adapter.
pub fn bootstrap() -> Result<ExecutionContext, BootstrapError> {
    bootstrap_with_storage(crate::layer::LayerStorage::in_memory())
}

/// Bootstrap with caller-provided storage. Used by `bootstrap_persistent`
/// (RocksDB-backed) and tests that need a particular storage configuration.
pub fn bootstrap_with_storage(
    storage: crate::layer::LayerStorage,
) -> Result<ExecutionContext, BootstrapError> {
    let core = load_layer(
        "core",
        include_str!("../../../ontologies/core/core-ontology.json"),
        None,
        storage.clone(),
    )?;

    // eigentt-type-fragment layer (D47) — chain-mirrored EigenTT type
    // language for axiom and theorem statements (D46 §10 axioms,
    // future propositional institutions). Depends only on core (uses
    // core:string, core:integer, core:InductiveType, core:InductiveCtor,
    // core:InductiveArgType).
    let eigentt_type = load_layer(
        "eigentt-type-fragment",
        include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
        Some(core),
        storage.clone(),
    )?;

    let program = load_layer(
        "program",
        include_str!("../../../ontologies/program/program-ontology.json"),
        Some(eigentt_type),
        storage.clone(),
    )?;

    let reflection = load_layer(
        "reflection",
        include_str!("../../../ontologies/reflection/reflection-ontology.json"),
        Some(program),
        storage.clone(),
    )?;

    // obo layer — shared OBO meta-vocabulary used by the obograph
    // importer (M9.2). Declares the four synonym Properties
    // (`has_exact_synonym`, `has_related_synonym`,
    // `has_broad_synonym`, `has_narrow_synonym`) and the
    // `inverseOf` RBox axiom so imported GO / ChEBI / etc. layers
    // can reference them without re-declaring. Depends on
    // reflection because the entries themselves carry
    // `is_a: [DeclaredResource]` and `declared_by`.
    let obo = load_layer(
        "obo",
        include_str!("../../../ontologies/obo/obo-meta-ontology.json"),
        Some(reflection),
        storage.clone(),
    )?;

    let institution = load_layer(
        "institution",
        include_str!("../../../ontologies/institution/institution-ontology.json"),
        Some(obo),
        storage.clone(),
    )?;

    let runtime = load_layer(
        "runtime",
        include_str!("../../../ontologies/runtime/runtime-substrate-ontology.json"),
        Some(institution),
        storage.clone(),
    )?;

    // formulas: layer (Phase 19d.0.d / D32 §4-5) — declares
    // FormulaTerm (the shared symbol-algebra term language across
    // every numerical institution) and the v1 operator catalog
    // (arithmetic, unary numeric, comparisons, derivative). Sits
    // above runtime since `Operator.operator_signature` uses the
    // `core:inductive` data type added in 19d.0.b.
    let formulas = load_layer(
        "formulas",
        include_str!("../../../ontologies/formulas/formulas-ontology.json"),
        Some(runtime),
        storage.clone(),
    )?;

    // lean-expressions layer (Phase 20a.2 / D40) — chain-mirrored
    // Lean expression form: LeanName + LeanLevel + LeanLevelList +
    // LeanExpr InductiveTypes. Foundational shapes the Lean
    // verification institution's LeanProofTerm.proposition field
    // refers to. Lives in the bootstrap chain so subsequent
    // committed resources (LeanProofTerm instances, the
    // Lean-institution ontology layer) can reference these types
    // without needing the Lean institution to be registered first.
    let lean_expressions = load_layer(
        "lean-expressions",
        include_str!("../../../ontologies/lean/lean-expressions.eigon.json"),
        Some(formulas),
        storage.clone(),
    )?;

    // lean-runtime-classes layer (Phase 20a.5a / D28 §10.3) —
    // declares the authoring-side resource classes the Lean
    // language runtime owns: `LeanProject` / `LeanPackage` (subclasses
    // of `RuntimePackage`), `LeanPackagePin` (subclass of
    // `RuntimePackagePin`), and `LeanEnvironment` (subclass of
    // `RuntimeEnvironment` with `lean_permitted_axioms` /
    // `lean_unpermitted_axiom_hard_error` / `lake_lockfile_hash`).
    // Sits above `lean-expressions` purely for organisational
    // grouping (all Lean ontology layers cluster here); the
    // technical dependency is on `runtime` further down.
    let lean_runtime_classes = load_layer(
        "lean-runtime-classes",
        include_str!("../../../ontologies/lean/lean-runtime-classes.eigon.json"),
        Some(lean_expressions),
        storage.clone(),
    )?;

    // lean-institution layer (Phase 20a.4 / D28) — declares the
    // Lean 4 verification institution and its v1 surface:
    // LeanProofTerm / LeanProofPayload / LeanAxiomList resource
    // classes, the qc_proof_check QueryClass (AutoOnLoad + OnDemand
    // over LeanProofTerm → Verdict), and the ef_lean_proof_payload
    // ExportFormat. Sits above lean-runtime-classes so future
    // additions (e.g. a LeanProofTerm.environment property
    // referencing `lean:LeanEnvironment`) resolve cleanly.
    let lean_institution = load_layer(
        "lean-institution",
        include_str!("../../../ontologies/lean/lean-institution.eigon.json"),
        Some(lean_runtime_classes),
        storage.clone(),
    )?;

    // reasoning layer (D39 Phase 8) — the Justification Logic
    // institution's chain artifacts: the four `ChainWitness.Is*As`
    // predicates (D49 §6), `JustificationTerm` + `JustifiedBy`
    // indexed inductives, `ReasoningSentence` + `VerifiedPropositionView`
    // resource classes, `EntailmentRequest` + `ConsistencyRequest`
    // query-input classes, the `reasoning_institution` resource +
    // three QueryClass declarations + `ef_justification` ExportFormat.
    // Loaded from ESL source (not JSON) — compiled at bootstrap to
    // keep `ontologies/reasoning/reasoning.esl` as the single source
    // of truth. Depends on every layer below it: core (primitives),
    // eigentt-type-fragment (TypeExpr), reflection (DerivedResource +
    // canonical_proposition), institution (Institution / QueryClass /
    // ExportFormat / dispatch roles).
    let reasoning = load_esl_layer(
        "reasoning",
        include_str!("../../../ontologies/reasoning/reasoning.esl"),
        Some(lean_institution),
        storage.clone(),
    )?;

    // D52 Phase 5: measurement-statistics ontology — universal claim
    // schema, SampleSet sum-types, seven smart-constructor macros
    // (SingleSampleEstimate, IID, Paired, Factorial, RCBD, SplitPlot,
    // RepeatedMeasures), StatisticalAnalysisPlan + MethodComparisonAnalysisPlan
    // classes, the four §7 opinionated-stance marker classes
    // (PopulationLevel / MeasurementLevel / ImpossibilityWitness), and
    // two QueryClass registrations bound to the matching in-process
    // StatisticsInstitution (linked via cli/src/main.rs). Stacked
    // above reasoning so the notebook layer sees both the reasoning
    // and statistics ontologies in its parent chain — the D52 → D39
    // composition the docs describe (D52 emits canonical_proposition,
    // D39 cites via DerivedEvidence + JustifiedBy.derived consuming
    // the D49 IsDerivedAs witness) works out of the box.
    let statistics = load_esl_layer(
        "statistics",
        include_str!("../../../ontologies/statistics/statistics.esl"),
        Some(reasoning),
        storage.clone(),
    )?;

    let notebook = load_layer(
        "notebook",
        include_str!("../../../ontologies/notebook/notebook-ontology.json"),
        Some(statistics),
        storage.clone(),
    )?;

    Ok(ExecutionContext::new(
        notebook,
        "working",
        ExecutionMode::ReadWrite,
        storage,
    ))
}

/// Bootstrap against a persistent backend (D13 §4).
///
/// Two paths:
///
/// - **SEED** (empty backend): run the normal in-memory bootstrap,
///   then commit each of the five embedded ontology layers
///   (core → program → reflection → institution → runtime → notebook) to the
///   backend in parent→child order, record the seed manifest, and
///   create the `main` branch pointing at the notebook layer.
/// - **RESUME** (backend has a `main` branch): verify the stored seed
///   manifest matches the current embedded ontologies' SHA-256 hashes;
///   if it does, rehydrate the layer chain from the backend. If it
///   doesn't, return a `ManifestDrift` error with actionable detail.
///
/// Phase 14g: presence of `branch:main` is the seed-vs-resume
/// discriminator. The pre-Phase-14 single-`head` pointer is gone;
/// branches are the only sanctioned head-pointer surface.
pub fn bootstrap_persistent(
    backend: Arc<dyn crate::storage::PersistentBackend>,
) -> Result<ExecutionContext, BootstrapError> {
    match backend
        .get_branch("main")
        .map_err(|e| BootstrapError::Storage(format!("get_branch(main): {e}")))?
    {
        Some(_) => resume_from_backend(backend),
        None => seed_backend(backend),
    }
}

// --- SEED / RESUME helpers ---

const SEED_MANIFEST_KEY: &str = "seed_manifest_v1";

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Stamp `meta:schema_version` + `meta:last_writer_version` +
/// `meta:schema_history` (empty Vec) on a freshly-seeded DB. Called
/// at the end of `seed_backend`. D24 §2.1.
fn stamp_schema_version_seed(
    backend: &dyn crate::storage::PersistentBackend,
) -> Result<(), BootstrapError> {
    use crate::storage::version::{
        encode_schema_version, MigrationRecord, LAST_WRITER_VERSION_KEY, SCHEMA_HISTORY_KEY,
        SCHEMA_VERSION, SCHEMA_VERSION_KEY,
    };

    backend
        .put_meta(SCHEMA_VERSION_KEY, &encode_schema_version(SCHEMA_VERSION))
        .map_err(|e| BootstrapError::Storage(format!("put_meta(schema_version): {e}")))?;

    backend
        .put_meta(
            LAST_WRITER_VERSION_KEY,
            env!("CARGO_PKG_VERSION").as_bytes(),
        )
        .map_err(|e| BootstrapError::Storage(format!("put_meta(last_writer_version): {e}")))?;

    let empty_history: Vec<MigrationRecord> = Vec::new();
    let mut history_bytes = Vec::new();
    ciborium::into_writer(&empty_history, &mut history_bytes)
        .map_err(|e| BootstrapError::Storage(format!("encode schema_history: {e}")))?;
    backend
        .put_meta(SCHEMA_HISTORY_KEY, &history_bytes)
        .map_err(|e| BootstrapError::Storage(format!("put_meta(schema_history): {e}")))?;
    Ok(())
}

/// Read + validate `meta:schema_version` against the kernel's compiled
/// expectation. Returns `Ok(())` when the DB is at the expected
/// version (no migration needed); `Err` otherwise. Migration handling
/// (running registered `Migration` impls) is performed by the caller
/// when the registry is non-empty — Phase 14 ships an empty registry
/// so any `from < SCHEMA_VERSION` currently surfaces as
/// `NoMigrationPath`. D24 §2.
fn check_and_migrate_schema_version(
    backend: &dyn crate::storage::PersistentBackend,
) -> Result<(), BootstrapError> {
    use crate::storage::version::{
        decode_schema_version, encode_schema_version, MigrationRecord, MigrationRegistry,
        LAST_WRITER_VERSION_KEY, SCHEMA_HISTORY_KEY, SCHEMA_VERSION, SCHEMA_VERSION_KEY,
    };

    let stored_bytes = backend
        .get_meta(SCHEMA_VERSION_KEY)
        .map_err(|e| BootstrapError::Storage(format!("get_meta(schema_version): {e}")))?;

    let stored = match stored_bytes {
        Some(bytes) => {
            decode_schema_version(&bytes).map_err(BootstrapError::SchemaVersionCorrupt)?
        }
        None => return Err(BootstrapError::SchemaVersionAbsent),
    };

    if stored == SCHEMA_VERSION {
        return Ok(());
    }
    if stored > SCHEMA_VERSION {
        return Err(BootstrapError::SchemaTooNew {
            stored,
            kernel: SCHEMA_VERSION,
        });
    }

    // Stored < SCHEMA_VERSION: run registered migrations stored → SCHEMA_VERSION.
    let registry = MigrationRegistry::default();
    if !registry.has_path(stored, SCHEMA_VERSION) {
        return Err(BootstrapError::NoMigrationPath {
            from: stored,
            to: SCHEMA_VERSION,
        });
    }

    let mut current = stored;
    while current < SCHEMA_VERSION {
        let migration = registry
            .get(current)
            .ok_or(BootstrapError::NoMigrationPath {
                from: current,
                to: SCHEMA_VERSION,
            })?;
        let next = migration.to_version();

        tracing::info!(
            { field::OPERATION } = operation::BOOTSTRAP_LOAD,
            from = current,
            to = next,
            description = migration.description(),
            "applying schema migration"
        );

        migration
            .apply(backend)
            .map_err(|e| BootstrapError::MigrationFailed {
                from: current,
                to: next,
                message: e.to_string(),
            })?;

        // Re-stamp version + last writer + append history. Three
        // separate meta writes; the version stamp last so a crash mid-
        // append leaves the DB at the pre-migration version (the
        // history is append-only and idempotent at re-run).
        let history_bytes = backend
            .get_meta(SCHEMA_HISTORY_KEY)
            .map_err(|e| BootstrapError::Storage(format!("get_meta(schema_history): {e}")))?
            .unwrap_or_default();
        let mut history: Vec<MigrationRecord> = if history_bytes.is_empty() {
            Vec::new()
        } else {
            ciborium::from_reader(history_bytes.as_slice())
                .map_err(|e| BootstrapError::Storage(format!("decode schema_history: {e}")))?
        };
        history.push(MigrationRecord {
            from: current,
            to: next,
            applied_at_ms: now_millis(),
            kernel_version: env!("CARGO_PKG_VERSION").to_string(),
        });
        let mut new_history = Vec::new();
        ciborium::into_writer(&history, &mut new_history)
            .map_err(|e| BootstrapError::Storage(format!("encode schema_history: {e}")))?;
        backend
            .put_meta(SCHEMA_HISTORY_KEY, &new_history)
            .map_err(|e| BootstrapError::Storage(format!("put_meta(schema_history): {e}")))?;

        backend
            .put_meta(
                LAST_WRITER_VERSION_KEY,
                env!("CARGO_PKG_VERSION").as_bytes(),
            )
            .map_err(|e| BootstrapError::Storage(format!("put_meta(last_writer_version): {e}")))?;

        // Version stamp last — see comment above.
        backend
            .put_meta(SCHEMA_VERSION_KEY, &encode_schema_version(next))
            .map_err(|e| BootstrapError::Storage(format!("put_meta(schema_version): {e}")))?;

        current = next;
    }

    Ok(())
}

fn embedded_ontologies() -> [(&'static str, &'static str); 14] {
    [
        (
            "core",
            include_str!("../../../ontologies/core/core-ontology.json"),
        ),
        (
            "eigentt-type-fragment",
            include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
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
            "obo",
            include_str!("../../../ontologies/obo/obo-meta-ontology.json"),
        ),
        (
            "institution",
            include_str!("../../../ontologies/institution/institution-ontology.json"),
        ),
        (
            "runtime",
            include_str!("../../../ontologies/runtime/runtime-substrate-ontology.json"),
        ),
        (
            "formulas",
            include_str!("../../../ontologies/formulas/formulas-ontology.json"),
        ),
        (
            "lean-expressions",
            include_str!("../../../ontologies/lean/lean-expressions.eigon.json"),
        ),
        (
            "lean-runtime-classes",
            include_str!("../../../ontologies/lean/lean-runtime-classes.eigon.json"),
        ),
        (
            "lean-institution",
            include_str!("../../../ontologies/lean/lean-institution.eigon.json"),
        ),
        // The reasoning layer is sourced from ESL rather than JSON,
        // but the manifest's job is content-drift detection — hashing
        // the raw source bytes is what we want either way. A change
        // to reasoning.esl bumps the manifest, forcing a SEED rebuild
        // against a stale persistent DB. Same applies to the
        // statistics layer below.
        (
            "reasoning",
            include_str!("../../../ontologies/reasoning/reasoning.esl"),
        ),
        (
            "statistics",
            include_str!("../../../ontologies/statistics/statistics.esl"),
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
    backend: Arc<dyn crate::storage::PersistentBackend>,
) -> Result<ExecutionContext, BootstrapError> {
    // Build the embedded ontologies in memory (reusing the existing path)
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

    // Phase 14g: create the `main` branch pointing at the notebook layer.
    // This is the only head-pointer surface from Phase 14 onward —
    // bootstrap_persistent's seed-vs-resume discriminator above keys off
    // the presence of `branch:main`.
    let storage = crate::layer::LayerStorage::with_persistent(Arc::clone(&backend));
    crate::lattice::update_branch(
        "main",
        None,
        ctx.head().id().clone(),
        crate::lattice::ConflictPolicy::AllowTrivial,
        storage,
        backend.as_ref(),
    )
    .map_err(|e| BootstrapError::Storage(format!("create main branch: {e}")))?;

    backend
        .put_meta(SEED_MANIFEST_KEY, &current_manifest())
        .map_err(|e| BootstrapError::Storage(format!("put_meta(manifest): {e}")))?;

    // D24: stamp schema version + writer + empty history.
    stamp_schema_version_seed(backend.as_ref())?;

    Ok(ctx)
}

fn resume_from_backend(
    backend: Arc<dyn crate::storage::PersistentBackend>,
) -> Result<ExecutionContext, BootstrapError> {
    // D24 §2.2: schema version check first. Refuse to boot before
    // touching the chain on a version mismatch — there's no point
    // validating ontology fingerprints against a DB whose shape we
    // can't safely walk. Migrations (when registered) run inside
    // this call.
    check_and_migrate_schema_version(backend.as_ref())?;

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

    // Phase 14g: load chain from `branch:main` head. The discriminator
    // in `bootstrap_persistent` already verified the branch exists.
    let main_head = backend
        .get_branch("main")
        .map_err(|e| BootstrapError::Storage(format!("get_branch(main): {e}")))?
        .ok_or_else(|| {
            BootstrapError::Storage(
                "branch:main present in discriminator but absent in resume — concurrent delete?"
                    .into(),
            )
        })?;
    let info = backend
        .load_chain_from(&main_head)
        .map_err(|e| BootstrapError::Storage(format!("load_chain_from: {e}")))?
        .ok_or_else(|| BootstrapError::Storage("branch:main pointed at unknown layer".into()))?;

    // Storage backed by the live `PersistentBackend`. Cold-cache reads
    // hit RocksDB on demand; no separate warming step needed.
    let storage = crate::layer::LayerStorage::with_persistent(backend);

    let head = crate::layer::build_chain(info, storage.clone());

    Ok(ExecutionContext::new(
        head,
        "working",
        ExecutionMode::ReadWrite,
        storage,
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
        // (on top of lean-institution → lean-runtime-classes →
        // lean-expressions → formulas → runtime → institution →
        // reflection → program → core).
        // formulas inserted at Phase 19d.0.d / D32 §4 so FormulaTerm
        // and the operator catalog ride above the runtime substrate
        // ontology. lean-expressions inserted at Phase 20a.2 / D40
        // for the chain-mirrored Lean expression form.
        // lean-runtime-classes inserted at Phase 20a.5a / D28 §10.3
        // to declare LeanProject / LeanEnvironment subclasses.
        // lean-institution inserted at Phase 20a.4 / D28 to declare
        // the LeanProofTerm class + qc_proof_check QueryClass.
        // reasoning inserted at D39 Phase 8 to declare the
        // Justification Logic institution and its chain artifacts.
        // statistics inserted at D52 Phase 5 to declare the
        // Measurement Statistics institution and its chain artifacts.
        assert!(!ctx.head().is_root());
        let statistics = ctx.head().parent().unwrap();
        assert!(!statistics.is_root());
        let reasoning = statistics.parent().unwrap();
        assert!(!reasoning.is_root());
        let lean_institution = reasoning.parent().unwrap();
        assert!(!lean_institution.is_root());
        let lean_runtime_classes = lean_institution.parent().unwrap();
        assert!(!lean_runtime_classes.is_root());
        let lean_expressions = lean_runtime_classes.parent().unwrap();
        assert!(!lean_expressions.is_root());
        let formulas = lean_expressions.parent().unwrap();
        assert!(!formulas.is_root());
        let runtime = formulas.parent().unwrap();
        assert!(!runtime.is_root());
        let institution = runtime.parent().unwrap();
        assert!(!institution.is_root());
        let obo = institution.parent().unwrap();
        assert!(!obo.is_root());
        let reflection = obo.parent().unwrap();
        assert!(!reflection.is_root());
        let program = reflection.parent().unwrap();
        assert!(!program.is_root());
        let eigentt_type = program.parent().unwrap();
        assert!(!eigentt_type.is_root());
        // Core layer (parent of eigentt-type-fragment) should be root.
        assert!(eigentt_type.parent().unwrap().is_root());
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
    fn can_resolve_eigentt_type_expr() {
        // D47: the chain-mirrored EigenTT type fragment lives at
        // urn:eigenius:eigentt:TypeExpr and is loaded just above the core
        // layer.
        let ctx = bootstrap().unwrap();
        let iri = Iri::parse("urn:eigenius:eigentt:TypeExpr").unwrap();
        let resolved = ctx
            .resolve(&iri)
            .expect("should resolve eigentt:TypeExpr from the eigentt-type-fragment layer");
        let is_a = resolved.is_a();
        let inductive_type_iri = Iri::parse("urn:eigenius:core:InductiveType").unwrap();
        assert!(
            is_a.iter().any(|i| i == &inductive_type_iri),
            "eigentt:TypeExpr should be an InductiveType; is_a = {is_a:?}"
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

    /// D43 §3.1 — `core:TextIndex` and `core:VectorIndex` Class
    /// declarations resolve from the core ontology. M1 deliverable.
    #[test]
    fn can_resolve_d43_index_classes() {
        let ctx = bootstrap().unwrap();
        for class in ["TextIndex", "VectorIndex"] {
            let iri = Iri::parse(&format!("urn:eigenius:core:{class}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve D43 index class {class}"
            );
        }
    }

    /// D43 §3.1 — enum Classes that narrow `vec_distance` /
    /// `vec_strategy` / `vec_embedding_policy` resolve from the core
    /// ontology, plus each of their declared Resource instances.
    #[test]
    fn can_resolve_d43_enum_classes_and_instances() {
        let ctx = bootstrap().unwrap();
        for class in ["DistanceMetric", "VectorStrategy", "EmbeddingPolicy"] {
            let iri = Iri::parse(&format!("urn:eigenius:core:{class}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve D43 enum class {class}"
            );
        }

        for (prefix, instance) in [
            ("distances", "cosine"),
            ("distances", "l2"),
            ("distances", "dot"),
            ("strategies", "flat"),
            ("strategies", "hnsw"),
            ("strategies", "auto"),
            ("embedding_policies", "eager_on_load"),
            ("embedding_policies", "lazy_on_query"),
            ("embedding_policies", "manual"),
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:core:{prefix}:{instance}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve D43 enum instance {prefix}:{instance}"
            );
        }
    }

    /// D43 §3.1 — Properties carried by TextIndex / VectorIndex
    /// Resources resolve from the core ontology.
    #[test]
    fn can_resolve_d43_index_properties() {
        let ctx = bootstrap().unwrap();
        for prop in [
            "target_property",
            "text_analyzer",
            "vec_model",
            "vec_dim",
            "vec_distance",
            "vec_strategy",
            "vec_hnsw_m",
            "vec_hnsw_ef_construction",
            "vec_embedding_policy",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:core:{prop}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve D43 index property {prop}"
            );
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
            "ExportFormat",
            "ImportFormat",
            "QueryClass",
            "Comorphism",
            "Verdict",
            "RuntimeKind",
            "DispatchRole",
        ] {
            let iri = Iri::parse(&format!("urn:eigenius:institution:{class}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve institution class {class}"
            );
        }
    }

    #[test]
    fn can_resolve_dispatch_roles() {
        let ctx = bootstrap().unwrap();
        for role in ["on_demand", "auto_on_load", "decidable"] {
            let iri =
                Iri::parse(&format!("urn:eigenius:institution:dispatch_roles:{role}")).unwrap();
            assert!(
                ctx.resolve(&iri).is_some(),
                "should resolve dispatch role {role}"
            );
        }
    }

    #[test]
    fn bootstrap_includes_reasoning_layer_artifacts() {
        // D39 Phase 8 — confirm the reasoning layer's load_esl_layer
        // call produced the expected chain artifacts: the 4 ChainWitness
        // predicates, the 2 indexed inductives (JustificationTerm +
        // JustifiedBy), the 2 resource classes (ReasoningSentence +
        // VerifiedPropositionView), the 2 query-request classes
        // (EntailmentRequest + ConsistencyRequest), the institution
        // resource, the 3 QueryClasses, and the ExportFormat.
        let ctx = bootstrap().unwrap();
        for iri in [
            "urn:eigenius:reasoning:ChainWitness:IsDeclaredAs",
            "urn:eigenius:reasoning:ChainWitness:IsObservedAs",
            "urn:eigenius:reasoning:ChainWitness:IsDerivedAs",
            "urn:eigenius:reasoning:ChainWitness:IsVerifiedAs",
            "urn:eigenius:reasoning:JustificationTerm",
            "urn:eigenius:reasoning:JustifiedBy",
            "urn:eigenius:reasoning:ReasoningSentence",
            "urn:eigenius:reasoning:VerifiedPropositionView",
            "urn:eigenius:reasoning:EntailmentRequest",
            "urn:eigenius:reasoning:ConsistencyRequest",
            "urn:eigenius:reasoning:reasoning_institution",
            "urn:eigenius:reasoning:qc_validate_justification",
            "urn:eigenius:reasoning:qc_entailment_query",
            "urn:eigenius:reasoning:qc_consistency_check",
            "urn:eigenius:reasoning:ef_justification",
        ] {
            let parsed = Iri::parse(iri).unwrap();
            assert!(
                ctx.resolve(&parsed).is_some(),
                "bootstrap should resolve reasoning-layer artifact `{iri}`"
            );
        }
    }

    // --- D24 schema-version tests ---

    use crate::storage::memory::MemoryPersistentBackend;
    use crate::storage::version::{
        decode_schema_version, encode_schema_version, MigrationRecord, LAST_WRITER_VERSION_KEY,
        SCHEMA_HISTORY_KEY, SCHEMA_VERSION, SCHEMA_VERSION_KEY,
    };

    #[test]
    fn seed_stamps_schema_version() {
        let backend: Arc<dyn crate::storage::PersistentBackend> =
            Arc::new(MemoryPersistentBackend::new());
        let _ctx = bootstrap_persistent(Arc::clone(&backend)).unwrap();

        let stored = backend
            .get_meta(SCHEMA_VERSION_KEY)
            .unwrap()
            .expect("schema_version stamped on seed");
        assert_eq!(decode_schema_version(&stored).unwrap(), SCHEMA_VERSION);

        let writer = backend
            .get_meta(LAST_WRITER_VERSION_KEY)
            .unwrap()
            .expect("last_writer_version stamped on seed");
        assert_eq!(
            std::str::from_utf8(&writer).unwrap(),
            env!("CARGO_PKG_VERSION")
        );

        let history_bytes = backend
            .get_meta(SCHEMA_HISTORY_KEY)
            .unwrap()
            .expect("schema_history stamped on seed");
        let history: Vec<MigrationRecord> =
            ciborium::from_reader(history_bytes.as_slice()).unwrap();
        assert!(history.is_empty(), "fresh seed has no migration history");
    }

    #[test]
    fn resume_succeeds_when_version_matches() {
        let backend: Arc<dyn crate::storage::PersistentBackend> =
            Arc::new(MemoryPersistentBackend::new());

        // Seed.
        let _ = bootstrap_persistent(Arc::clone(&backend)).unwrap();
        // Resume — same backend, same version.
        let _ = bootstrap_persistent(Arc::clone(&backend)).unwrap();
    }

    #[test]
    fn resume_refuses_when_version_absent_on_non_empty_db() {
        let backend: Arc<dyn crate::storage::PersistentBackend> =
            Arc::new(MemoryPersistentBackend::new());

        // Seed, then strip the schema_version key to simulate a
        // pre-marker DB.
        let _ = bootstrap_persistent(Arc::clone(&backend)).unwrap();
        backend.delete_meta(SCHEMA_VERSION_KEY).unwrap();

        let err = match bootstrap_persistent(Arc::clone(&backend)) {
            Err(e) => e,
            Ok(_) => panic!("expected error, got success"),
        };
        assert!(
            matches!(err, BootstrapError::SchemaVersionAbsent),
            "expected SchemaVersionAbsent, got {err:?}"
        );
    }

    #[test]
    fn resume_refuses_when_version_too_new() {
        let backend: Arc<dyn crate::storage::PersistentBackend> =
            Arc::new(MemoryPersistentBackend::new());
        let _ = bootstrap_persistent(Arc::clone(&backend)).unwrap();

        // Stamp a version higher than the kernel expects.
        backend
            .put_meta(
                SCHEMA_VERSION_KEY,
                &encode_schema_version(SCHEMA_VERSION + 5),
            )
            .unwrap();

        let err = match bootstrap_persistent(Arc::clone(&backend)) {
            Err(e) => e,
            Ok(_) => panic!("expected error, got success"),
        };
        match err {
            BootstrapError::SchemaTooNew { stored, kernel } => {
                assert_eq!(stored, SCHEMA_VERSION + 5);
                assert_eq!(kernel, SCHEMA_VERSION);
            }
            other => panic!("expected SchemaTooNew, got {other:?}"),
        }
    }

    #[test]
    fn resume_refuses_when_no_migration_path() {
        // Phase 14 ships SCHEMA_VERSION = 1 and an empty registry, so
        // any stored < SCHEMA_VERSION (impossible until v2 lands) would
        // have no path. Simulate by stamping v0 even though that's not
        // a real version — the boot check decodes 0 as a u32 and looks
        // for a 0→1 migration, finds none, refuses.
        let backend: Arc<dyn crate::storage::PersistentBackend> =
            Arc::new(MemoryPersistentBackend::new());
        let _ = bootstrap_persistent(Arc::clone(&backend)).unwrap();

        // Force stored back to 0 to simulate a pre-v1 DB that
        // somehow got stamped (real pre-v1 DBs would hit
        // SchemaVersionAbsent above).
        backend
            .put_meta(SCHEMA_VERSION_KEY, &encode_schema_version(0))
            .unwrap();

        let err = match bootstrap_persistent(Arc::clone(&backend)) {
            Err(e) => e,
            Ok(_) => panic!("expected error, got success"),
        };
        match err {
            BootstrapError::NoMigrationPath { from, to } => {
                assert_eq!(from, 0);
                assert_eq!(to, SCHEMA_VERSION);
            }
            other => panic!("expected NoMigrationPath, got {other:?}"),
        }
    }

    #[test]
    fn resume_refuses_when_version_corrupt() {
        let backend: Arc<dyn crate::storage::PersistentBackend> =
            Arc::new(MemoryPersistentBackend::new());
        let _ = bootstrap_persistent(Arc::clone(&backend)).unwrap();

        // Stamp a 3-byte (wrong-length) value.
        backend
            .put_meta(SCHEMA_VERSION_KEY, &[0x00, 0x00, 0x01])
            .unwrap();

        let err = match bootstrap_persistent(Arc::clone(&backend)) {
            Err(e) => e,
            Ok(_) => panic!("expected error, got success"),
        };
        assert!(
            matches!(err, BootstrapError::SchemaVersionCorrupt(_)),
            "expected SchemaVersionCorrupt, got {err:?}"
        );
    }

    /// Confirm that a kernel-emitted Verdict resource (the shape
    /// AutoOnLoad fires-and-emits at every StatisticalAnalysisPlan /
    /// ReasoningSentence commit per D14 §5.6) validates cleanly. The
    /// resource carries `core:ctor_name` to record which Verdict ctor
    /// (Holds / Fails / Undecidable) the institution returned — same
    /// property declared on InductiveCtor for declared-ctor names.
    /// The property's `domain` must include `Verdict` so the
    /// retroactive-validate pass (which walks the merged chain view
    /// looking for property carriers) doesn't trip on the AutoOnLoad-
    /// emitted Verdict the second time the same notebook cell runs.
    #[test]
    fn kernel_emitted_verdict_validates_cleanly() {
        use crate::ontology::resource::Value;
        use crate::ontology::well_known as wk;
        use crate::validation::Validator;
        use std::sync::Arc;

        let ctx = bootstrap().unwrap();
        let mut builder =
            crate::layer::LayerBuilder::new("test-verdict", Some(Arc::clone(ctx.head())));
        let mut r = crate::ontology::resource::Resource::new(
            Iri::parse("urn:eigenius:verdict:test:abc").unwrap(),
        );
        r.set(
            Iri::parse(wk::IS_A).unwrap(),
            Value::Array(vec![
                Value::String(wk::VERDICT.to_string()),
                Value::String(wk::DERIVED_RESOURCE.to_string()),
            ]),
        );
        r.set(
            Iri::parse(wk::CTOR_NAME).unwrap(),
            Value::String("Holds".to_string()),
        );
        r.set(
            Iri::parse("urn:eigenius:institution:verdict_subject").unwrap(),
            Value::String("urn:eigenius:test:subject".to_string()),
        );
        builder.add_resource(r).unwrap();
        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let domain_violations: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == crate::validation::ValidationRule::DomainViolation)
            .collect();
        assert!(
            domain_violations.is_empty(),
            "Verdict resource must not trigger DomainViolation on `core:ctor_name` — the \
             AutoOnLoad-emitted Verdict reaches every commit's retroactive-validate pass; \
             got: {domain_violations:#?}"
        );
    }

    /// D52 Phase 5 — confirm the statistics-layer macros (smart
    /// constructors like `stats:SingleSampleEstimate`) are reachable
    /// from notebook-cell ESL via `esl::compile_full`. This is the
    /// load-bearing path for the [stats-and-reasoning notebook]
    /// (../../../notebooks/examples/stats-and-reasoning.json): the
    /// server-side `parse_resources` calls `compile_full` with the
    /// branch's head layer, which seeds the compiler's macro table
    /// from `collect_macros_from_layer` walking parent layers. If
    /// statistics.esl's macros didn't land in the bootstrap, or if
    /// the server's compile path bypassed the layer, this would
    /// reject with `macro X is not declared in this file`.
    #[test]
    fn notebook_can_invoke_statistics_macros_via_compile_full() {
        use std::sync::Arc;
        let ctx = bootstrap().unwrap();
        let head = ctx.head();
        let index = Arc::new(crate::institution::registry::InstitutionIndex::default());

        let sample_set_cell = r#"
namespace reflection = "urn:eigenius:reflection";
namespace stats      = "urn:eigenius:measurements";
namespace screen     = "urn:eigenius:demo:screen";

resource screen:m_eig0291_sampleset : stats:SampleSetResource {
    reflection:source      = "instrument-log:kinase-glo-plate-2026-03-11";
    reflection:observed_at = "2026-03-11T10:18:42Z";

    stats:sample_set_value = stats:SingleSampleEstimate(
        [78.0, 82.0, 85.0, 88.0, 91.0, 86.0],
        BiologicalReplication(),
    );
}
"#;

        let resources =
            crate::esl::compile_full(sample_set_cell, index, head).unwrap_or_else(|errs| {
                panic!(
                    "stats:SingleSampleEstimate macro should resolve from the bootstrapped \
                     statistics layer via compile_full; got: {errs:?}"
                )
            });
        assert!(
            !resources.is_empty(),
            "should compile to at least one resource"
        );
    }
}
