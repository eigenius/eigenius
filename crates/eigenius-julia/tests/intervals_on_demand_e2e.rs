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

//! Phase 19d.2 / D14 §6.2 — OnDemand kernel-side dispatch e2e against
//! the IntervalArithmetic institution.
//!
//! Step B of the Phase 19d sequence (after Comorphism formalisation in
//! Step A): proves the kernel's `Institution::query` dispatch surface
//! reaches a real external Julia institution end-to-end. The
//! AutoOnLoad path was already wired in 19a.6; this test pins the
//! OnDemand path against the new `qc_compute_bounds` QueryClass.
//!
//! What is exercised:
//!
//! 1. The kernel's `InstitutionRuntime` registers a real institution
//!    that wraps the substrate's `dispatch_external_institution` (in
//!    production this wrapper is `ExternalInstitution`, which routes
//!    through orchestrator gRPC; the test uses an in-process adapter
//!    so we don't have to stand up an orchestrator process).
//! 2. `InstitutionIndex::from_layer` derives the OnDemand QueryClass
//!    declaration from the chain — the same indexing pass FIBER and
//!    AutoOnLoad use.
//! 3. Dispatch via `runtime.get(inst).query(handler_iri, &input, &ctx)`
//!    — the same call FIBER eval makes after resolving the QueryClass.
//! 4. The Julia handler `compute_bounds_for_request(req::BoundsRequest)`
//!    decodes the composite mirror struct, runs interval arithmetic
//!    over the FormulaTerm, and returns a `BoundedBy` — assertions
//!    confirm the response is a tight enclosure of `sin(x) + 0.5`
//!    over `[0, π/2]`.
//!
//! The test exercises *both* dispatch surfaces back-to-back against the
//! same env image:
//!
//! - **Direct call** — `Institution::query(handler_iri, &input, &ctx)`
//!   with a hand-built embedded `BoundsRequest`. Pins the institution-
//!   runtime boundary itself.
//! - **Textual FIBER** — `execute_with(FIBER cap:qc_compute_bounds {
//!   expr: <iri>, domain: <iri> } AS ?bound RETURN …)` against a chain
//!   carrying anchor `expr` / `domain` resources. Pins the kernel's
//!   IRI-dereference pass on FIBER param values (Phase 19d.2 follow-on)
//!   and the `?bound.lower` dot-path projection's overlay-aware
//!   resolution.

use eigenius_julia::mirror_gen::{mirror_to_resource, JuliaMirrorGenerator};
use eigenius_julia::JuliaLanguageRuntime;
use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::registry::{DispatchRole, InstitutionIndex};
use eigenius_kernel::institution::runtime::{Institution, InstitutionRuntime, QueryOutcome};
use eigenius_kernel::lattice::commit_layer_default;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::storage::memory::MemoryPersistentBackend;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::facade::SubstrateDispatcher;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::mirror_generator::{MirrorGenerationRequest, MirrorGenerator};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const INTERVALS_ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/intervals/declarations/intervals-ontology.eigon.json"
);
const INTERVALS_INSTITUTION_JSON: &str = include_str!(
    "../../../julia/institutions/intervals/declarations/intervals-institution.eigon.json"
);
const SYMBOLICS_ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/symbolics/declarations/symbolics-ontology.eigon.json"
);
// Phase 19f.1: symbolics ontology now references jump:VariableBound and
// jump:Constraint via SymbolicsToJuMPInput's framing properties, so JuMP
// ontology must precede symbolics on the chain.
const JUMP_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/jump/declarations/jump-ontology.eigon.json");
const INTERVALS_HANDLER_PROJECT_TOML: &str =
    include_str!("../../../julia/institutions/intervals/EigeniusIntervals/Project.toml");
const INTERVALS_HANDLER_SOURCE_JL: &str = include_str!(
    "../../../julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl"
);
const FORMULAS_ONTOLOGY_JSON: &str =
    include_str!("../../../ontologies/formulas/formulas-ontology.json");

const INSTITUTION_IRI: &str = "urn:eigenius:institutions:intervals";
const QC_COMPUTE_BOUNDS_IRI: &str = "urn:eigenius:intervals:query_classes:qc_compute_bounds";
const SIGNATURE_IRI: &str = "urn:eigenius:intervals:signatures:compute_bounds_for_request";
const ENV_IRI: &str = "urn:eigenius:test:on_demand:env";
const BOUNDS_REQUEST_CLASS_IRI: &str = "urn:eigenius:intervals:BoundsRequest";
const BOUNDED_BY_CLASS_IRI: &str = "urn:eigenius:intervals:BoundedBy";
const SYMBOLIC_EXPRESSION_CLASS_IRI: &str = "urn:eigenius:symbolics:SymbolicExpression";
const HANDLER_METHOD_NAME: &str = "compute_bounds_for_request";
const HANDLER_PACKAGE_NAME: &str = "EigeniusIntervals";
const BASE_IMAGE_TAG: &str = "julia:1.12-bookworm";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

// ─── Chain construction ─────────────────────────────────────────────────

const ANCHOR_EXPR_IRI: &str = "urn:eigenius:test:on_demand:expr";
const ANCHOR_DOMAIN_IRI: &str = "urn:eigenius:test:on_demand:domain";

/// Build the head layer with bootstrap + intervals + symbolics +
/// intervals-institution declarations committed, plus chain-anchored
/// `expr` and `domain` resources used by the textual FIBER dispatch
/// path. The textual path passes their IRIs as FIBER param values;
/// the kernel's IRI-dereference pass embeds them before they flow to
/// the institution.
fn build_chain() -> (Arc<Layer>, LayerStorage) {
    // D41 Phase G migration: bootstrap with a memory-backed
    // `PersistentBackend` so layer commits go through
    // `commit_layer_default` — the D41 supported single-layer-commit
    // surface.
    let backend = Arc::new(MemoryPersistentBackend::new());
    let storage = LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
    let mut ctx = eigenius_kernel::bootstrap::bootstrap_with_storage(storage).expect("bootstrap");
    // Commit order:
    //   - jump ontology first because symbolics now references
    //     jump:VariableBound / jump:Constraint via class_types
    //     (Phase 19f.1 / SymbolicsToJuMPInput).
    //   - symbolics ontology before intervals because intervals'
    //     BoundsRequest declares `expr` with class_types:
    //     [SymbolicExpression].
    for (label, json) in [
        ("jump_ontology", JUMP_ONTOLOGY_JSON),
        ("symbolics_ontology", SYMBOLICS_ONTOLOGY_JSON),
        ("intervals_ontology", INTERVALS_ONTOLOGY_JSON),
        ("intervals_institution", INTERVALS_INSTITUTION_JSON),
    ] {
        for r in eigon_json::parse_document(json).expect("parse") {
            ctx.add_resource(r).expect("add_resource");
        }
        let working = ctx.take_working(label).expect("take_working");
        let layer =
            commit_layer_default(working, ctx.storage().clone(), backend.as_ref()).expect("commit");
        ctx.advance_head(layer, label).expect("advance_head");
    }

    // Anchor a SymbolicExpression and a BoundedBy on the chain so the
    // textual FIBER syntax can reference them by IRI. Routed through
    // `commit_layer_default` (no AutoOnLoad — no InstitutionRuntime is
    // wired during chain build, so even the orchestrator's
    // `WithInstitutions` pipeline wouldn't fire any gates here).
    let mut expr = Resource::new(iri(ANCHOR_EXPR_IRI));
    expr.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(SYMBOLIC_EXPRESSION_CLASS_IRI))]),
    );
    expr.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("on_demand_test_expr".into()),
    );
    expr.set(
        iri("urn:eigenius:symbolics:term"),
        Value::Json(serde_json::json!({
            "ctor": "App",
            "args": [
                {
                    "ctor": "App",
                    "args": [
                        {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
                        {
                            "ctor": "App",
                            "args": [
                                {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:sin"]},
                                {"ctor": "Var", "args": ["x"]}
                            ]
                        }
                    ]
                },
                {"ctor": "LitFloat", "args": [0.5]}
            ]
        })),
    );
    ctx.add_resource(expr).expect("add chain expr");

    let mut domain = Resource::new(iri(ANCHOR_DOMAIN_IRI));
    domain.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(BOUNDED_BY_CLASS_IRI))]),
    );
    domain.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("on_demand_test_domain".into()),
    );
    domain.set(iri("urn:eigenius:intervals:value"), Value::Float(0.0));
    domain.set(iri("urn:eigenius:intervals:lower"), Value::Float(0.0));
    domain.set(
        iri("urn:eigenius:intervals:upper"),
        Value::Float(std::f64::consts::FRAC_PI_2),
    );
    ctx.add_resource(domain).expect("add chain domain");
    let chain_inputs_working = ctx
        .take_working("chain_inputs")
        .expect("take_working chain_inputs");
    let chain_inputs_layer = commit_layer_default(
        chain_inputs_working,
        ctx.storage().clone(),
        backend.as_ref(),
    )
    .expect("commit chain inputs");
    ctx.advance_head(chain_inputs_layer, "chain_inputs")
        .expect("advance_head chain_inputs");

    (Arc::clone(ctx.head()), ctx.storage().clone())
}

/// Generate a Julia mirror seeded on the OnDemand QueryClass's input
/// class. The closure walker pulls in `BoundsRequest`'s fields
/// (`SymbolicExpression`, `BoundedBy`) and transitively the FormulaTerm
/// inductive + operator catalog.
fn build_on_demand_mirror() -> Resource {
    // Pool the three on-disk ontology files into a CrossChain — same
    // shape the cross-institution probe uses. Walking the bootstrapped
    // chain layer would also work, but the file-pool path keeps the
    // closure mechanically identical to the probe and side-steps any
    // bootstrap-only metadata that doesn't belong in the mirror.
    struct CrossChain {
        resources: HashMap<Iri, Resource>,
    }
    impl ChainAccessor for CrossChain {
        fn resolve(&self, _: &Iri, target: &Iri) -> Option<Resource> {
            self.resources.get(target).cloned()
        }
        fn is_ancestor_or_equal(&self, _: &Iri, _: &Iri) -> bool {
            true
        }
        fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
            true
        }
    }
    let mut resources = HashMap::new();
    for json in [
        JUMP_ONTOLOGY_JSON,
        INTERVALS_ONTOLOGY_JSON,
        SYMBOLICS_ONTOLOGY_JSON,
        FORMULAS_ONTOLOGY_JSON,
    ] {
        for r in eigon_json::parse_document(json).expect("parse") {
            if let Some(id) = r.id() {
                resources.insert(id.clone(), r);
            }
        }
    }
    let chain = CrossChain { resources };

    let g = JuliaMirrorGenerator::new();
    let layer_iri = iri("urn:eigenius:test:on_demand:layer");
    let seed = vec![
        iri(BOUNDS_REQUEST_CLASS_IRI),
        iri(BOUNDED_BY_CLASS_IRI),
        iri(SYMBOLIC_EXPRESSION_CLASS_IRI),
    ];
    let out = g
        .generate(&MirrorGenerationRequest {
            source_layer: &layer_iri,
            seed_classes: &seed,
            chain: &chain,
        })
        .expect("on-demand mirror generation");
    assert!(
        out.mirrored_classes
            .iter()
            .any(|i| i.as_str() == BOUNDS_REQUEST_CLASS_IRI),
        "mirror closure must include BoundsRequest"
    );
    mirror_to_resource(&g, &out, &layer_iri, Some("1970-01-01T00:00:00Z"))
}

/// Build a `RuntimePackage` Resource for EigeniusIntervals, sourced
/// from the on-disk handler files. Same encoder as the cross-
/// institution probe.
fn build_handler_package() -> Resource {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    fn encode(input: &[u8]) -> String {
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0];
            let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
            out.push(ALPHABET[(b0 >> 2) as usize] as char);
            out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }
    let mut r = Resource::new(iri(
        "urn:eigenius:test:on_demand:handler-package:EigeniusIntervals",
    ));
    r.set(
        iri("urn:eigenius:runtime:package_name"),
        Value::String(HANDLER_PACKAGE_NAME.to_string()),
    );
    r.set(
        iri("urn:eigenius:runtime:manifest"),
        Value::String(INTERVALS_HANDLER_PROJECT_TOML.to_string()),
    );
    r.set(
        iri("urn:eigenius:runtime:source_tree"),
        Value::Json(serde_json::json!([{
            "path": "src/EigeniusIntervals.jl",
            "content_base64": encode(INTERVALS_HANDLER_SOURCE_JL.as_bytes()),
        }])),
    );
    r
}

// ─── Input construction ─────────────────────────────────────────────────

/// Build the embedded `BoundsRequest(expr=sin(x)+0.5, domain=[0, π/2])`
/// resource the kernel hands to `Institution::query`.
fn build_bounds_request_input() -> Resource {
    let mut expr = Resource::new_embedded();
    expr.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(SYMBOLIC_EXPRESSION_CLASS_IRI))]),
    );
    expr.set(
        iri("urn:eigenius:symbolics:term"),
        Value::Json(serde_json::json!({
            "ctor": "App",
            "args": [
                {
                    "ctor": "App",
                    "args": [
                        {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
                        {
                            "ctor": "App",
                            "args": [
                                {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:sin"]},
                                {"ctor": "Var", "args": ["x"]}
                            ]
                        }
                    ]
                },
                {"ctor": "LitFloat", "args": [0.5]}
            ]
        })),
    );

    let mut domain = Resource::new_embedded();
    domain.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(BOUNDED_BY_CLASS_IRI))]),
    );
    domain.set(iri("urn:eigenius:intervals:value"), Value::Float(0.0));
    domain.set(iri("urn:eigenius:intervals:lower"), Value::Float(0.0));
    domain.set(
        iri("urn:eigenius:intervals:upper"),
        Value::Float(std::f64::consts::FRAC_PI_2),
    );

    let mut req = Resource::new_embedded();
    req.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(BOUNDS_REQUEST_CLASS_IRI))]),
    );
    req.set(
        iri("urn:eigenius:intervals:expr"),
        Value::Embedded(Box::new(expr)),
    );
    req.set(
        iri("urn:eigenius:intervals:domain"),
        Value::Embedded(Box::new(domain)),
    );
    req
}

// ─── Substrate-backed Institution wrapper ───────────────────────────────

/// Test-only adapter that exposes the substrate's
/// `dispatch_external_institution` as the kernel's `Institution`
/// interface. Production wiring is `ExternalInstitution`, which routes
/// through the orchestrator's gRPC `dispatch_external` RPC; the test
/// skips the gRPC layer because there's no orchestrator process to
/// talk to. The path *under* the gRPC layer is identical (same
/// substrate dispatcher, same Julia worker, same mirror codec), so the
/// test still proves the kernel-side dispatch shape works against a
/// live institution.
struct SubstrateBackedInstitution {
    institution_iri: Iri,
    handler_method: String,
    handler_signature: String,
    env_iri: String,
    image_digest: String,
    dispatcher: Mutex<SubstrateDispatcher>,
}

impl Institution for SubstrateBackedInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.institution_iri
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        _: &Resource,
        _: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "substrate-backed test institution does not implement extract_typed for {procedure_iri}"
        )))
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        _: &Val,
        _: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "substrate-backed test institution does not implement reify for {procedure_iri}"
        )))
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        if procedure_iri.as_str() != self.handler_signature {
            return Err(InstitutionError::UnknownType(format!(
                "substrate-backed institution: unknown procedure `{procedure_iri}` (expected `{}`)",
                self.handler_signature
            )));
        }
        let input_cbor = eigon_cbor::serialize_resource(input);
        let dispatcher = self.dispatcher.lock().expect("dispatcher mutex");
        let outcome = dispatcher
            .dispatch_external_institution(
                "julia",
                &self.env_iri,
                &self.image_digest,
                &self.handler_method,
                &self.handler_signature,
                &[input_cbor],
            )
            .map_err(|e| InstitutionError::ComputationFailed(format!("substrate dispatch: {e}")))?;
        let output = eigon_cbor::parse_resource_lenient(&outcome.output_cbor)
            .map_err(|e| InstitutionError::ComputationFailed(format!("decode output_cbor: {e}")))?;
        Ok(QueryOutcome {
            output,
            derivations: Vec::new(),
            partial_invocation: None,
        })
    }
}

// ─── Environment / skip gates ───────────────────────────────────────────

fn fresh_depot(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-julia-on-demand-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create depot");
    dir
}

fn julia_project_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("julia")
        .join("runtime-worker")
        .canonicalize()
        .expect("julia/runtime-worker/ must exist relative to eigenius-julia's Cargo.toml")
}

fn is_docker_available() -> bool {
    Path::new("/var/run/docker.sock").exists()
}

fn ensure_base_image_pinned() -> Result<String, String> {
    let pull = std::process::Command::new("docker")
        .args(["pull", "--quiet", BASE_IMAGE_TAG])
        .output()
        .map_err(|e| format!("`docker pull` failed: {e}"))?;
    if !pull.status.success() {
        return Err(format!(
            "docker pull {BASE_IMAGE_TAG} exited {}: {}",
            pull.status,
            String::from_utf8_lossy(&pull.stderr)
        ));
    }
    let inspect = std::process::Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{index .RepoDigests 0}}",
            BASE_IMAGE_TAG,
        ])
        .output()
        .map_err(|e| format!("`docker image inspect` failed: {e}"))?;
    if !inspect.status.success() {
        return Err(format!(
            "docker image inspect failed: {}",
            String::from_utf8_lossy(&inspect.stderr)
        ));
    }
    let pinned = String::from_utf8_lossy(&inspect.stdout).trim().to_string();
    if !pinned.contains('@') {
        return Err(format!(
            "expected RepoDigests entry like `julia@sha256:...`, got `{pinned}`"
        ));
    }
    Ok(if pinned.contains('/') {
        pinned
    } else {
        format!("docker.io/library/{pinned}")
    })
}

fn skip_unless_full_environment() -> Option<String> {
    if !is_docker_available() {
        return Some("Docker socket unavailable".into());
    }
    if !eigenius_runtime_substrate::is_buildah_available() {
        return Some("buildah unavailable".into());
    }
    None
}

// ─── The test ───────────────────────────────────────────────────────────

#[ignore = "heavy E2E: Julia env image build for OnDemand FIBER dispatch."]
#[test]
fn on_demand_dispatch_invokes_julia_institution_via_kernel_runtime() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping intervals on-demand e2e: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping (could not pin base image): {e}");
            return;
        }
    };

    // 1. Build the chain — so the InstitutionIndex can derive the
    //    OnDemand QueryClass declaration. This proves the chain shapes
    //    we authored in Steps 1-3 above are actually discoverable by
    //    the same indexing pass FIBER and AutoOnLoad use.
    let (head, storage) = build_chain();
    let exec_ctx = ExecutionContext::new(
        Arc::clone(&head),
        "on_demand_test",
        ExecutionMode::ReadOnly,
        storage.clone(),
    );

    let (index, index_errors) = InstitutionIndex::from_layer(&head);
    assert!(
        index_errors.is_empty(),
        "InstitutionIndex.from_layer must succeed: {index_errors:?}"
    );

    let qc_iri = iri(QC_COMPUTE_BOUNDS_IRI);
    let qc_entry = index
        .query_class(&qc_iri)
        .expect("qc_compute_bounds must be indexed");
    assert!(
        qc_entry.dispatch_roles.contains(&DispatchRole::OnDemand),
        "qc_compute_bounds must declare OnDemand dispatch role"
    );
    assert_eq!(
        qc_entry.institution_ref.as_str(),
        INSTITUTION_IRI,
        "qc_compute_bounds must point at the IntervalArithmetic institution"
    );
    assert_eq!(qc_entry.query_handler.as_str(), SIGNATURE_IRI);

    // 2. Build the env image with EigeniusIntervals + the BoundsRequest-
    //    aware mirror baked in.
    let depot = fresh_depot("e2e");
    let spawner = match eigenius_runtime_substrate::spawner::service::DockerServiceSpawner::new(
        eigenius_runtime_substrate::spawner::DockerSpawnerConfig::new(depot.clone()),
    ) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("skipping (DockerServiceSpawner construction failed): {e}");
            return;
        }
    };
    let project_dir = julia_project_dir();
    let runtime = Arc::new(JuliaLanguageRuntime::new(
        project_dir,
        pinned_base,
        spawner.clone(),
        depot.clone(),
    ));

    let mirror = build_on_demand_mirror();
    let handler_pkg = build_handler_package();
    let env = Resource::new_embedded();
    let runtime_for_dispatch: Box<dyn LanguageRuntime> = Box::new(runtime.clone());
    let digest =
        match runtime_for_dispatch.build_environment_image(&env, &[handler_pkg], Some(&mirror)) {
            Ok(d) => d,
            Err(e) => {
                let _ = runtime.drain();
                let _ = std::fs::remove_dir_all(&depot);
                panic!("build_environment_image: {e}");
            }
        };

    let mut dispatcher = SubstrateDispatcher::new();
    dispatcher
        .register_language_runtime(runtime_for_dispatch)
        .expect("register julia runtime");

    // Bring EigeniusIntervals into Main so the worker's
    // `Core.eval(Main, fn_symbol)` lookup resolves
    // `compute_bounds_for_request`. Same one-shot pattern the
    // intervals e2e and the cross-institution probe use.
    let setup_arg = {
        let mut arg = Resource::new_embedded();
        arg.set(
            iri("urn:eigenius:runtime:language"),
            Value::String("julia".into()),
        );
        arg.set(
            iri("urn:eigenius:runtime:source"),
            Value::String("begin; using EigeniusIntervals; nothing; end".into()),
        );
        eigon_cbor::serialize_resource(&arg)
    };
    if let Err(e) = dispatcher.dispatch_run_runtime_script(&[], &setup_arg) {
        let _ = runtime.drain();
        let _ = std::fs::remove_dir_all(&depot);
        panic!("setup script (using EigeniusIntervals) failed: {e:?}");
    }

    // 3. Register the substrate-backed institution and dispatch via the
    //    kernel's Institution::query interface — the same interface
    //    FIBER eval calls into.
    let inst = SubstrateBackedInstitution {
        institution_iri: iri(INSTITUTION_IRI),
        handler_method: HANDLER_METHOD_NAME.to_string(),
        handler_signature: SIGNATURE_IRI.to_string(),
        env_iri: ENV_IRI.to_string(),
        image_digest: digest.as_str().to_string(),
        dispatcher: Mutex::new(dispatcher),
    };
    let mut inst_runtime = InstitutionRuntime::new();
    inst_runtime
        .register(Box::new(inst))
        .expect("register substrate-backed institution");

    let institution = inst_runtime
        .get(&qc_entry.institution_ref)
        .expect("registered institution must be retrievable");

    let outcome = match institution.query(
        &qc_entry.query_handler,
        &build_bounds_request_input(),
        &exec_ctx,
    ) {
        Ok(o) => o,
        Err(e) => {
            // Drain the runtime via the InstitutionRuntime → can't,
            // since we moved it. Fall through to the panic; the depot
            // cleanup runs out-of-band when the test process exits.
            panic!("institution.query failed: {e}");
        }
    };

    // 4. The output is a BoundedBy whose [lower, upper] should bracket
    //    [0.5, 1.5] — `sin(x) + 0.5` over `[0, π/2]` ranges exactly
    //    over that interval. Interval arithmetic gives a slightly wider
    //    bound; we assert containment, not point-equality.
    let lower = outcome
        .output
        .get(&iri("urn:eigenius:intervals:lower"))
        .and_then(Value::as_float)
        .expect("output carries lower");
    let upper = outcome
        .output
        .get(&iri("urn:eigenius:intervals:upper"))
        .and_then(Value::as_float)
        .expect("output carries upper");
    assert!(
        lower <= 0.5,
        "lower bound must enclose 0.5; got lower={lower}, upper={upper}"
    );
    assert!(
        upper >= 1.5,
        "upper bound must enclose 1.5; got lower={lower}, upper={upper}"
    );
    assert!(
        lower >= 0.0 && upper <= 2.0,
        "interval must be tight (within [0, 2]) — got [{lower}, {upper}]"
    );

    // 5. Textual FIBER dispatch — exercises the kernel's IRI-dereference
    //    pass on FIBER param values (the chain-anchored `expr` and
    //    `domain` flow in as IRI strings; the kernel embeds them
    //    before serialising) plus the dot-path projection's
    //    overlay-aware resolution (?bound.lower / ?bound.upper read
    //    off the FIBER response which lives in the transient overlay,
    //    not the chain).
    let fiber_query = format!(
        r#"
USING INSTITUTION "{INSTITUTION_IRI}" AS cap
USING NAMESPACE "urn:eigenius:intervals:query_classes:"
FIBER cap:qc_compute_bounds {{
    expr: "{ANCHOR_EXPR_IRI}",
    domain: "{ANCHOR_DOMAIN_IRI}"
}} AS ?bound
RETURN [] {{
    lower: ?bound.lower,
    upper: ?bound.upper
}}
"#
    );

    let fiber_runtime = eigenius_kernel::query::evaluate::FiberRuntime {
        index: Some(&index),
        runtime: Some(&inst_runtime),
        components: None,
        overlay: None,
        ctx: Some(&exec_ctx),
        similarity: None,
        embedders: None,
        embedding_cache: None,
        vector_segment_cache: None,
    };

    let document = match eigenius_kernel::query::execute_with(&fiber_query, &head, fiber_runtime) {
        Ok(d) => d,
        Err(errors) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("textual FIBER query failed: {errors:?}");
        }
    };

    // The document follows D2 Appendix A: a `ResultSet` resource
    // carries an embedded `rows` array. Each row's properties map
    // synthesized RETURN-item IRIs to the projected values. Find the
    // ResultSet, walk its single row, and pull the two Float values.
    let result_set = document
        .iter()
        .find(|r| {
            r.get(&iri("urn:eigenius:core:is_a"))
                .and_then(|v| v.as_iri_array().first().cloned())
                .map(|i| i.as_str() == "urn:eigenius:query:ResultSet")
                .unwrap_or(false)
        })
        .expect("textual FIBER query must produce a ResultSet");
    let rows = match result_set.get(&iri("urn:eigenius:query:rows")) {
        Some(Value::Array(a)) => a,
        other => panic!("ResultSet missing rows array — got {other:?}"),
    };
    assert_eq!(
        rows.len(),
        1,
        "expected exactly one row, got {}",
        rows.len()
    );
    let row = match &rows[0] {
        Value::Embedded(r) => r,
        other => panic!("row must be embedded — got {other:?}"),
    };
    let mut floats: Vec<f64> = row
        .properties()
        .values()
        .filter_map(|v| {
            if let Value::Float(f) = v {
                Some(*f)
            } else {
                None
            }
        })
        .collect();
    floats.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        floats.len(),
        2,
        "row must carry exactly two Float values — got {floats:?}"
    );
    let (fiber_lower, fiber_upper) = (floats[0], floats[1]);
    assert!(
        fiber_lower <= 0.5 && fiber_upper >= 1.5,
        "FIBER-path bounds must enclose [0.5, 1.5]; got [{fiber_lower}, {fiber_upper}]"
    );
    // Sanity: both dispatch surfaces returned the same interval —
    // they're calling the same handler against the same env image.
    assert!(
        (fiber_lower - lower).abs() < 1e-12 && (fiber_upper - upper).abs() < 1e-12,
        "direct and FIBER paths must return the same interval: \
         direct=[{lower}, {upper}], fiber=[{fiber_lower}, {fiber_upper}]"
    );

    // 6. Sanity: the LayerStorage we pulled from the bootstrap context
    //    is reachable; flush nothing — we only used the head for index
    //    derivation, not for new commits.
    let _ = storage;
    let _ = LayerBuilder::new("on_demand_unused", Some(Arc::clone(&head)));

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}
