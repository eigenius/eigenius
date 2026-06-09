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

//! Phase 19h.1 / D27 §4.4.4 — Catalyst → DiffEq comorphism e2e.
//!
//! Demonstrates the full cross-institution pipeline that motivates
//! the FormulaTerm-typed RHS surface (D32 §6):
//!
//! 1. Author a Catalyst `ReactionNetwork` for the simple two-species
//!    reaction `A → B` (rate `k`). Closed-form solution at `t=1`
//!    with `k = 1, A(0) = 1, B(0) = 0` is `A(1) = e^-1` and
//!    `B(1) = 1 - e^-1` — hand-verifiable.
//! 2. Build a `CatalystToOdeInput` carrying the network plus initial
//!    conditions, parameter values, and time span.
//! 3. Dispatch Catalyst's OnDemand `qc_cat_to_ode` (handler:
//!    `compile_to_ode`). The handler calls `Catalyst.netstoichmat *
//!    Catalyst.oderatelaw.(reactions(rn))` to get the symbolic
//!    per-species RHS, translates each to FormulaTerm via
//!    `num_to_formula`, packs into an `OdeProblem` mirror struct.
//! 4. Decode the resulting `OdeProblem` as a chain Resource.
//! 5. Build an `OdeSolution` carrying the decoded OdeProblem
//!    (embedded), the algorithm + tolerances, and the closed-form
//!    final state `[e^-1, 1 - e^-1]`.
//! 6. Dispatch DiffEq's AutoOnLoad `validate_solution`. The handler
//!    decodes the OdeProblem's FormulaTerm RHS, builds a numerical
//!    closure via `formula_to_value`, integrates, and per-component
//!    compares the integrator's final state against the claim. Holds.
//!
//! Single env image with both EigeniusCatalyst + EigeniusDiffEq
//! handler packages baked in. Same Julia worker dispatches both
//! `compile_to_ode` and `validate_solution`; the InstitutionRuntime
//! has two SubstrateBackedInstitution entries (one per institution
//! IRI) sharing the same `SubstrateDispatcher`.
//!
//! Skipped on hosts without buildah / Docker / a pullable Julia
//! base image. Cold env build is heavy (~15 min — Catalyst pulls
//! MTK + SymbolicUtils + DiffEqBase + a long SciML dep tail, plus
//! OrdinaryDiffEq + SciMLBase on top).

use eigenius_julia::mirror_gen::{mirror_to_resource, JuliaMirrorGenerator};
use eigenius_julia::JuliaLanguageRuntime;
use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::{Institution, InstitutionRuntime, QueryOutcome};
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::facade::SubstrateDispatcher;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::mirror_generator::{MirrorGenerationRequest, MirrorGenerator};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// ─── Source-of-truth artifacts ──────────────────────────────────────────

const CATALYST_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/catalyst/declarations/catalyst-ontology.eigon.json");
const DIFFEQ_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/diffeq/declarations/diffeq-ontology.eigon.json");
const FORMULAS_ONTOLOGY_JSON: &str =
    include_str!("../../../ontologies/formulas/formulas-ontology.json");

const CATALYST_HANDLER_PROJECT_TOML: &str =
    include_str!("../../../julia/institutions/catalyst/EigeniusCatalyst/Project.toml");
const CATALYST_HANDLER_SOURCE_JL: &str =
    include_str!("../../../julia/institutions/catalyst/EigeniusCatalyst/src/EigeniusCatalyst.jl");
const DIFFEQ_HANDLER_PROJECT_TOML: &str =
    include_str!("../../../julia/institutions/diffeq/EigeniusDiffEq/Project.toml");
const DIFFEQ_HANDLER_SOURCE_JL: &str =
    include_str!("../../../julia/institutions/diffeq/EigeniusDiffEq/src/EigeniusDiffEq.jl");

// ─── IRIs the test pins ─────────────────────────────────────────────────

const CATALYST_INST_IRI: &str = "urn:eigenius:institutions:catalyst";
const DIFFEQ_INST_IRI: &str = "urn:eigenius:institutions:diffeq";

const COMPILE_TO_ODE_SIG_IRI: &str = "urn:eigenius:catalyst:signatures:compile_to_ode";
const VALIDATE_SOLUTION_SIG_IRI: &str = "urn:eigenius:diffeq:signatures:validate_solution";

const CATALYST_TO_ODE_INPUT_CLASS: &str = "urn:eigenius:catalyst:CatalystToOdeInput";
const REACTION_NETWORK_CLASS: &str = "urn:eigenius:catalyst:ReactionNetwork";
const CONSERVATION_LAW_CLASS: &str = "urn:eigenius:catalyst:ConservationLaw";
const ODE_SOLUTION_CLASS: &str = "urn:eigenius:diffeq:OdeSolution";
const ODE_PROBLEM_CLASS: &str = "urn:eigenius:diffeq:OdeProblem";

const ENV_IRI: &str = "urn:eigenius:test:cat_to_diffeq:env";

const BASE_IMAGE_TAG: &str = "julia:1.12-bookworm";

// Two-species reaction network — closed-form solvable for hand-
// verification of the integrated final state.
//
//   k, A --> B
//
// With A(0)=1, B(0)=0, k=1, t=0..1:
//   A(1) = e^-1 ≈ 0.36787944117144233
//   B(1) = 1 - e^-1 ≈ 0.6321205588285577
const NETWORK_SOURCE: &str = "@reaction_network begin\n    k, A --> B\nend";
const TIME_SPAN_END: f64 = 1.0;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

// ─── Chain pool + mirror generation ─────────────────────────────────────

/// In-memory chain that pools resources from the catalyst, diffeq,
/// and formulas ontologies. The mirror generator's closure walker
/// resolves transitive references through this single accessor.
struct CrossChain {
    resources: HashMap<Iri, Resource>,
}

impl CrossChain {
    fn new() -> Self {
        let mut resources = HashMap::new();
        for json in [
            CATALYST_ONTOLOGY_JSON,
            DIFFEQ_ONTOLOGY_JSON,
            FORMULAS_ONTOLOGY_JSON,
        ] {
            for r in eigon_json::parse_document(json).expect("ontology must parse") {
                if let Some(id) = r.id() {
                    resources.insert(id.clone(), r);
                }
            }
        }
        Self { resources }
    }
}

impl ChainAccessor for CrossChain {
    fn resolve(&self, _claim_layer: &Iri, target: &Iri) -> Option<Resource> {
        self.resources.get(target).cloned()
    }
    fn is_ancestor_or_equal(&self, _: &Iri, _: &Iri) -> bool {
        true
    }
    fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
        true
    }
}

/// Generate the Julia mirror seeded on `[CatalystToOdeInput,
/// OdeSolution, ReactionNetwork, ConservationLaw]`. The closure pulls
/// in `OdeProblem` (via OdeSolution.problem), `RhsComponent` (via
/// OdeProblem.rhs), `FormulaTerm` (via RhsComponent.term), and the
/// operator catalog (via FormulaTerm's transitive references).
///
/// `ConservationLaw` is seeded even though this e2e exercises only
/// `compile_to_ode` and `validate_solution`: the EigeniusCatalyst
/// module unconditionally references `EigeniusMirror.ConservationLaw`
/// at top level (in `validate_conservation_law`'s method dispatch),
/// so the symbol must exist in the mirror or the package fails to
/// precompile.
fn build_mirror() -> Resource {
    let g = JuliaMirrorGenerator::new();
    let chain = CrossChain::new();
    let layer_iri = iri("urn:eigenius:test:cat_to_diffeq:layer");
    let seed = vec![
        iri(CATALYST_TO_ODE_INPUT_CLASS),
        iri(ODE_SOLUTION_CLASS),
        iri(REACTION_NETWORK_CLASS),
        iri(CONSERVATION_LAW_CLASS),
    ];
    let out = g
        .generate(&MirrorGenerationRequest {
            source_layer: &layer_iri,
            seed_classes: &seed,
            chain: &chain,
        })
        .expect("cross-institution mirror generation");
    // Sanity: the closure must include CatalystToOdeInput,
    // OdeProblem, OdeSolution, RhsComponent, ReactionNetwork,
    // ConservationLaw.
    for required in [
        CATALYST_TO_ODE_INPUT_CLASS,
        ODE_PROBLEM_CLASS,
        ODE_SOLUTION_CLASS,
        "urn:eigenius:diffeq:RhsComponent",
        REACTION_NETWORK_CLASS,
        CONSERVATION_LAW_CLASS,
    ] {
        assert!(
            out.mirrored_classes.iter().any(|i| i.as_str() == required),
            "mirror closure missing {required}"
        );
    }
    mirror_to_resource(&g, &out, &layer_iri, Some("1970-01-01T00:00:00Z"))
}

// ─── Handler packages ───────────────────────────────────────────────────

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(input: &[u8]) -> String {
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

fn build_handler_package(name: &str, project_toml: &str, source_jl: &str) -> Resource {
    let mut r = Resource::new(iri(&format!(
        "urn:eigenius:test:cat_to_diffeq:handler-package:{name}"
    )));
    r.set(
        iri("urn:eigenius:runtime:package_name"),
        Value::String(name.to_string()),
    );
    r.set(
        iri("urn:eigenius:runtime:manifest"),
        Value::String(project_toml.to_string()),
    );
    r.set(
        iri("urn:eigenius:runtime:source_tree"),
        Value::Json(serde_json::json!([{
            "path": format!("src/{name}.jl"),
            "content_base64": base64(source_jl.as_bytes()),
        }])),
    );
    r
}

// ─── CBOR builders ──────────────────────────────────────────────────────

/// Build the embedded `CatalystToOdeInput(network, ICs, params,
/// tspan)` resource the kernel hands to `compile_to_ode`. The
/// network is embedded inline (the chain validator's IRI-dereference
/// pass would do this anyway for an on-chain network).
fn build_catalyst_to_ode_input_cbor() -> Vec<u8> {
    let mut network = Resource::new_embedded();
    network.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(REACTION_NETWORK_CLASS))]),
    );
    network.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("ab_reaction".into()),
    );
    network.set(
        iri("urn:eigenius:catalyst:network_source"),
        Value::String(NETWORK_SOURCE.into()),
    );
    network.set(
        iri("urn:eigenius:catalyst:species_declared"),
        Value::Array(vec![Value::String("A".into()), Value::String("B".into())]),
    );
    network.set(
        iri("urn:eigenius:catalyst:parameters_declared"),
        Value::Array(vec![Value::String("k".into())]),
    );

    let mut req = Resource::new_embedded();
    req.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(CATALYST_TO_ODE_INPUT_CLASS))]),
    );
    req.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("ab_input".into()),
    );
    req.set(
        iri("urn:eigenius:catalyst:network"),
        Value::Embedded(Box::new(network)),
    );
    req.set(
        iri("urn:eigenius:catalyst:initial_conditions"),
        Value::Array(vec![Value::Float(1.0), Value::Float(0.0)]),
    );
    req.set(
        iri("urn:eigenius:catalyst:parameter_values"),
        Value::Array(vec![Value::Float(1.0)]),
    );
    req.set(
        iri("urn:eigenius:catalyst:time_span_start"),
        Value::Float(0.0),
    );
    req.set(
        iri("urn:eigenius:catalyst:time_span_end"),
        Value::Float(TIME_SPAN_END),
    );
    eigon_cbor::serialize_resource(&req)
}

/// Build an `OdeSolution` resource carrying the just-produced
/// `OdeProblem` embedded inline + the closed-form final state. The
/// gate's re-integration should land at the same final state within
/// tolerance.
fn build_ode_solution_cbor(problem: Resource) -> Vec<u8> {
    let e_inv = (-1.0f64).exp(); // e^-1 ≈ 0.36787944117144233
    let mut sol = Resource::new_embedded();
    sol.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(ODE_SOLUTION_CLASS))]),
    );
    sol.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("ab_solution".into()),
    );
    sol.set(
        iri("urn:eigenius:diffeq:problem"),
        Value::Embedded(Box::new(problem)),
    );
    sol.set(
        iri("urn:eigenius:diffeq:algorithm"),
        Value::String("Tsit5".into()),
    );
    sol.set(iri("urn:eigenius:diffeq:abstol"), Value::Float(1e-8));
    sol.set(iri("urn:eigenius:diffeq:reltol"), Value::Float(1e-8));
    sol.set(
        iri("urn:eigenius:diffeq:final_state"),
        Value::Array(vec![Value::Float(e_inv), Value::Float(1.0 - e_inv)]),
    );
    eigon_cbor::serialize_resource(&sol)
}

fn build_setup_argument(source: &str) -> Vec<u8> {
    let mut arg = Resource::new_embedded();
    arg.set(
        iri("urn:eigenius:runtime:language"),
        Value::String("julia".to_string()),
    );
    arg.set(
        iri("urn:eigenius:runtime:source"),
        Value::String(source.to_string()),
    );
    eigon_cbor::serialize_resource(&arg)
}

// ─── Substrate-backed Institution wrapper ───────────────────────────────

/// Same shape as `intervals_on_demand_e2e::SubstrateBackedInstitution`,
/// extended to support multiple institutions sharing one substrate
/// dispatcher (since both Catalyst and DiffEq run inside the same
/// Julia worker in this test). Each institution wrapper handles one
/// procedure IRI; the test instantiates two — one for `compile_to_ode`,
/// one for `validate_solution`.
struct SubstrateBackedInstitution {
    institution_iri: Iri,
    handler_method: String,
    handler_signature: String,
    env_iri: String,
    image_digest: String,
    dispatcher: Arc<Mutex<SubstrateDispatcher>>,
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
            "substrate-backed institution does not implement extract_typed for {procedure_iri}"
        )))
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        _: &Val,
        _: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "substrate-backed institution does not implement reify for {procedure_iri}"
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
    let dir = std::env::temp_dir().join(format!("substrate-julia-cat2diffeq-{pid}-{label}-{n}"));
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

// ─── The probe ──────────────────────────────────────────────────────────

#[ignore = "heavy E2E: builds a Julia env image (buildah + Pkg.precompile, ~30-90s cold). Run with `cargo test -- --include-ignored`."]
#[test]
fn catalyst_to_diffeq_e2e_via_kinase_style_pipeline() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping catalyst→diffeq e2e: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping (could not pin base image): {e}");
            return;
        }
    };

    // Bootstrap a minimal ExecutionContext for the Institution::query
    // calls. Resources don't actually flow through the layer in this
    // test (the ExportFormat handler is dispatched directly), but
    // ExecutionContext requires a head layer.
    let storage = eigenius_kernel::layer::LayerStorage::in_memory();
    let head = std::sync::Arc::new(
        eigenius_kernel::layer::LayerBuilder::new("cat2diffeq_test", None).build(storage.clone()),
    );
    let exec_ctx = ExecutionContext::new(
        std::sync::Arc::clone(&head),
        "cat2diffeq_test",
        ExecutionMode::ReadOnly,
        storage,
    );

    // Build the env image with BOTH handler packages baked in.
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

    let mirror = build_mirror();
    let catalyst_pkg = build_handler_package(
        "EigeniusCatalyst",
        CATALYST_HANDLER_PROJECT_TOML,
        CATALYST_HANDLER_SOURCE_JL,
    );
    let diffeq_pkg = build_handler_package(
        "EigeniusDiffEq",
        DIFFEQ_HANDLER_PROJECT_TOML,
        DIFFEQ_HANDLER_SOURCE_JL,
    );
    let env = Resource::new_embedded();
    let runtime_for_dispatch: Box<dyn LanguageRuntime> = Box::new(runtime.clone());
    let digest = match runtime_for_dispatch.build_environment_image(
        &env,
        &[catalyst_pkg, diffeq_pkg],
        Some(&mirror),
    ) {
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
    let dispatcher = Arc::new(Mutex::new(dispatcher));

    // Bring both handler modules into Main so the worker's
    // `Core.eval(Main, fn_symbol)` lookup resolves both
    // `compile_to_ode` and `validate_solution`.
    {
        let setup = "begin; using EigeniusCatalyst; using EigeniusDiffEq; nothing; end";
        let setup_arg = build_setup_argument(setup);
        let dispatcher = dispatcher.lock().expect("dispatcher mutex");
        if let Err(e) = dispatcher.dispatch_run_runtime_script(&[], &setup_arg) {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("setup script (using EigeniusCatalyst, EigeniusDiffEq) failed: {e:?}");
        }
    }

    // Register two SubstrateBackedInstitutions sharing the same
    // dispatcher. Catalyst dispatches `compile_to_ode`; DiffEq
    // dispatches `validate_solution`.
    let catalyst_inst = SubstrateBackedInstitution {
        institution_iri: iri(CATALYST_INST_IRI),
        handler_method: "compile_to_ode".to_string(),
        handler_signature: COMPILE_TO_ODE_SIG_IRI.to_string(),
        env_iri: ENV_IRI.to_string(),
        image_digest: digest.as_str().to_string(),
        dispatcher: dispatcher.clone(),
    };
    let diffeq_inst = SubstrateBackedInstitution {
        institution_iri: iri(DIFFEQ_INST_IRI),
        handler_method: "validate_solution".to_string(),
        handler_signature: VALIDATE_SOLUTION_SIG_IRI.to_string(),
        env_iri: ENV_IRI.to_string(),
        image_digest: digest.as_str().to_string(),
        dispatcher: dispatcher.clone(),
    };
    let mut inst_runtime = InstitutionRuntime::new();
    inst_runtime
        .register(Box::new(catalyst_inst))
        .expect("register Catalyst");
    inst_runtime
        .register(Box::new(diffeq_inst))
        .expect("register DiffEq");

    // Step 1: dispatch Catalyst's compile_to_ode.
    let input_cbor = build_catalyst_to_ode_input_cbor();
    // Plug the input directly into Institution::query — same shape
    // the kernel's FIBER eval would do once it's wired against this
    // institution.
    let catalyst = inst_runtime
        .get(&iri(CATALYST_INST_IRI))
        .expect("catalyst registered");
    let input_resource =
        eigon_cbor::parse_resource_lenient(&input_cbor).expect("decode input CBOR");
    let compile_outcome =
        match catalyst.query(&iri(COMPILE_TO_ODE_SIG_IRI), &input_resource, &exec_ctx) {
            Ok(o) => o,
            Err(e) => {
                let _ = runtime.drain();
                let _ = std::fs::remove_dir_all(&depot);
                panic!("compile_to_ode dispatch failed: {e}");
            }
        };

    // Verify the OdeProblem shape.
    let ode_problem = compile_outcome.output;
    let state_names = ode_problem
        .get(&iri("urn:eigenius:diffeq:state_names"))
        .and_then(|v| match v {
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .expect("OdeProblem must carry state_names");
    assert_eq!(
        state_names,
        vec!["A".to_string(), "B".to_string()],
        "state_names should match the network's species_declared"
    );
    let parameter_names = ode_problem
        .get(&iri("urn:eigenius:diffeq:parameter_names"))
        .and_then(|v| match v {
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .expect("OdeProblem must carry parameter_names");
    assert_eq!(parameter_names, vec!["k".to_string()]);
    let rhs = ode_problem
        .get(&iri("urn:eigenius:diffeq:rhs"))
        .and_then(|v| match v {
            Value::Array(items) => Some(items.clone()),
            _ => None,
        })
        .expect("OdeProblem must carry rhs");
    assert_eq!(
        rhs.len(),
        2,
        "rhs must have one RhsComponent per species (A, B)"
    );

    // Step 2: build OdeSolution claim against the produced
    // OdeProblem and dispatch DiffEq's validate_solution.
    let solution_cbor = build_ode_solution_cbor(ode_problem);
    let diffeq = inst_runtime
        .get(&iri(DIFFEQ_INST_IRI))
        .expect("diffeq registered");
    let solution_resource =
        eigon_cbor::parse_resource_lenient(&solution_cbor).expect("decode solution CBOR");
    let validate_outcome = match diffeq.query(
        &iri(VALIDATE_SOLUTION_SIG_IRI),
        &solution_resource,
        &exec_ctx,
    ) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("validate_solution dispatch failed: {e}");
        }
    };

    // Verify Holds.
    let ctor = validate_outcome
        .output
        .get(&iri("urn:eigenius:core:ctor_name"))
        .and_then(|v| v.as_str().map(str::to_owned))
        .expect("Verdict must carry ctor_name");
    assert_eq!(
        ctor, "Holds",
        "DiffEq should validate the integrated trajectory of the Catalyst-compiled OdeProblem"
    );

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}
