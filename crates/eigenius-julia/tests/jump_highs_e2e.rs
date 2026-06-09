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

//! Phase 19f / D27 §4.2 — JuMP-HiGHS institution e2e.
//!
//! Two test cases through the same env image, exercising both the
//! OnDemand `qc_jump_solve` path and the AutoOnLoad
//! `validate_optimum` path:
//!
//! 1. **LP**: `min x + 2y s.t. x + y ≤ 10, 0 ≤ x,y ≤ 10`. Optimum
//!    `(x,y) = (0,0)` with objective `0`. Hand-author a chain-typed
//!    `OptimisationProblem`, dispatch `solve_problem`, decode the
//!    returned `OptimisesTo`, build a re-claim from it, dispatch
//!    `validate_optimum` → assert Holds.
//!
//! 2. **QP**: `min (x-1)² + (y-2)² s.t. x + y == 2`. Optimum
//!    `(x,y) = (0.5, 1.5)` with objective `0.5`. Same flow as the LP,
//!    but the objective uses `pow(•, 2.0)` — exercises the smart-pow
//!    walker rule. HiGHS only accepts `QuadExpr` objectives, so the
//!    smart unrolling is what makes this case land at all.
//!
//! Skipped on hosts without buildah / Docker / a pullable Julia base
//! image. Cold env build is heavy (~5–8 min — JuMP + HiGHS_jll plus
//! the MathOptInterface tail).

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

const JUMP_ONTOLOGY_JSON: &str =
    include_str!("../../../julia/institutions/jump/declarations/jump-ontology.eigon.json");
const FORMULAS_ONTOLOGY_JSON: &str =
    include_str!("../../../ontologies/formulas/formulas-ontology.json");

const HANDLER_PROJECT_TOML: &str =
    include_str!("../../../julia/institutions/jump/EigeniusJuMPHiGHS/Project.toml");
const HANDLER_SOURCE_JL: &str =
    include_str!("../../../julia/institutions/jump/EigeniusJuMPHiGHS/src/EigeniusJuMPHiGHS.jl");

// ─── IRIs the test pins ─────────────────────────────────────────────────

const JUMP_HIGHS_INST_IRI: &str = "urn:eigenius:institutions:jump_highs";
const SOLVE_PROBLEM_SIG_IRI: &str = "urn:eigenius:jump_highs:signatures:solve_problem";
const VALIDATE_OPTIMUM_SIG_IRI: &str = "urn:eigenius:jump_highs:signatures:validate_optimum";

const OPTIMISATION_PROBLEM_CLASS: &str = "urn:eigenius:jump:OptimisationProblem";
const OPTIMISES_TO_CLASS: &str = "urn:eigenius:jump:OptimisesTo";
const VARIABLE_BOUND_CLASS: &str = "urn:eigenius:jump:VariableBound";
const CONSTRAINT_CLASS: &str = "urn:eigenius:jump:Constraint";

const ENV_IRI: &str = "urn:eigenius:test:jump_highs:env";

const BASE_IMAGE_TAG: &str = "julia:1.12-bookworm";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

// ─── FormulaTerm builders (chain Eigon-CBOR shape) ──────────────────────
//
// Same encoding as `intervals_on_demand_e2e.rs` and the rest of the
// FormulaTerm-consuming tests: bare `ctor` / `args` keys, recursively.

fn ft_var(name: &str) -> serde_json::Value {
    serde_json::json!({ "ctor": "Var", "args": [name] })
}

fn ft_lit(v: f64) -> serde_json::Value {
    serde_json::json!({ "ctor": "LitFloat", "args": [v] })
}

fn ft_op(iri_str: &str) -> serde_json::Value {
    serde_json::json!({ "ctor": "OpRef", "args": [iri_str] })
}

fn ft_app(head: serde_json::Value, arg: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "ctor": "App", "args": [head, arg] })
}

fn ft_binop(op_iri: &str, a: serde_json::Value, b: serde_json::Value) -> serde_json::Value {
    ft_app(ft_app(ft_op(op_iri), a), b)
}

fn ft_add(a: serde_json::Value, b: serde_json::Value) -> serde_json::Value {
    ft_binop("urn:eigenius:formulas:ops:add", a, b)
}
fn ft_sub(a: serde_json::Value, b: serde_json::Value) -> serde_json::Value {
    ft_binop("urn:eigenius:formulas:ops:sub", a, b)
}
fn ft_mul(a: serde_json::Value, b: serde_json::Value) -> serde_json::Value {
    ft_binop("urn:eigenius:formulas:ops:mul", a, b)
}
fn ft_pow(a: serde_json::Value, b: serde_json::Value) -> serde_json::Value {
    ft_binop("urn:eigenius:formulas:ops:pow", a, b)
}

// ─── ConstraintRelation builders ────────────────────────────────────────

fn cr_le() -> serde_json::Value {
    serde_json::json!({ "ctor": "LE" })
}
fn cr_eq() -> serde_json::Value {
    serde_json::json!({ "ctor": "EQ" })
}

// ─── Resource builders ──────────────────────────────────────────────────

fn build_variable_bound(name: &str, lower: Option<f64>, upper: Option<f64>) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(VARIABLE_BOUND_CLASS))]),
    );
    r.set(
        iri("urn:eigenius:jump:variable_name"),
        Value::String(name.to_string()),
    );
    if let Some(l) = lower {
        r.set(iri("urn:eigenius:jump:lower"), Value::Float(l));
    }
    if let Some(u) = upper {
        r.set(iri("urn:eigenius:jump:upper"), Value::Float(u));
    }
    r
}

fn build_constraint(lhs: serde_json::Value, relation: serde_json::Value, rhs: f64) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(CONSTRAINT_CLASS))]),
    );
    r.set(iri("urn:eigenius:jump:lhs"), Value::Json(lhs));
    r.set(iri("urn:eigenius:jump:relation"), Value::Json(relation));
    r.set(iri("urn:eigenius:jump:rhs"), Value::Float(rhs));
    r
}

fn build_optimisation_problem(
    short_name: &str,
    variable_names: &[&str],
    variable_bounds: Vec<Resource>,
    objective: serde_json::Value,
    sense: &str,
    constraints: Vec<Resource>,
) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(OPTIMISATION_PROBLEM_CLASS))]),
    );
    r.set(
        iri("urn:eigenius:core:short_name"),
        Value::String(short_name.to_string()),
    );
    r.set(
        iri("urn:eigenius:jump:variable_names"),
        Value::Array(
            variable_names
                .iter()
                .map(|s| Value::String(s.to_string()))
                .collect(),
        ),
    );
    if !variable_bounds.is_empty() {
        r.set(
            iri("urn:eigenius:jump:variable_bounds"),
            Value::Array(
                variable_bounds
                    .into_iter()
                    .map(|vb| Value::Embedded(Box::new(vb)))
                    .collect(),
            ),
        );
    }
    r.set(iri("urn:eigenius:jump:objective"), Value::Json(objective));
    r.set(iri("urn:eigenius:jump:sense"), Value::String(sense.into()));
    if constraints.is_empty() {
        r.set(iri("urn:eigenius:jump:constraints"), Value::Array(vec![]));
    } else {
        r.set(
            iri("urn:eigenius:jump:constraints"),
            Value::Array(
                constraints
                    .into_iter()
                    .map(|c| Value::Embedded(Box::new(c)))
                    .collect(),
            ),
        );
    }
    r
}

// Hand-build an OptimisesTo *claim* for the validate_optimum path.
// (When dispatching solve_problem, the worker constructs the
// OptimisesTo on its own and we read it back.)
fn build_optimises_to_claim(
    problem: Resource,
    termination_status: &str,
    objective_value: f64,
    variable_values: &[f64],
    abstol: f64,
    reltol: f64,
) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(OPTIMISES_TO_CLASS))]),
    );
    r.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("optimises_to_claim".into()),
    );
    r.set(
        iri("urn:eigenius:jump:problem"),
        Value::Embedded(Box::new(problem)),
    );
    r.set(
        iri("urn:eigenius:jump:termination_status"),
        Value::String(termination_status.into()),
    );
    r.set(
        iri("urn:eigenius:jump:objective_value"),
        Value::Float(objective_value),
    );
    r.set(
        iri("urn:eigenius:jump:variable_values"),
        Value::Array(variable_values.iter().map(|v| Value::Float(*v)).collect()),
    );
    r.set(iri("urn:eigenius:jump:abstol"), Value::Float(abstol));
    r.set(iri("urn:eigenius:jump:reltol"), Value::Float(reltol));
    r
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

// ─── Chain pool + mirror generation ─────────────────────────────────────

struct CrossChain {
    resources: HashMap<Iri, Resource>,
}

impl CrossChain {
    fn new() -> Self {
        let mut resources = HashMap::new();
        for json in [JUMP_ONTOLOGY_JSON, FORMULAS_ONTOLOGY_JSON] {
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

fn build_mirror() -> Resource {
    let g = JuliaMirrorGenerator::new();
    let chain = CrossChain::new();
    let layer_iri = iri("urn:eigenius:test:jump_highs:layer");
    let seed = vec![iri(OPTIMISATION_PROBLEM_CLASS), iri(OPTIMISES_TO_CLASS)];
    let out = g
        .generate(&MirrorGenerationRequest {
            source_layer: &layer_iri,
            seed_classes: &seed,
            chain: &chain,
        })
        .expect("mirror generation");
    for required in [
        OPTIMISATION_PROBLEM_CLASS,
        OPTIMISES_TO_CLASS,
        CONSTRAINT_CLASS,
        VARIABLE_BOUND_CLASS,
    ] {
        assert!(
            out.mirrored_classes.iter().any(|i| i.as_str() == required),
            "mirror closure missing {required}"
        );
    }
    mirror_to_resource(&g, &out, &layer_iri, Some("1970-01-01T00:00:00Z"))
}

// ─── Handler package ────────────────────────────────────────────────────

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
        "urn:eigenius:test:jump_highs:handler-package:{name}"
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

// ─── Substrate-backed Institution wrapper ───────────────────────────────

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
    let dir = std::env::temp_dir().join(format!("substrate-julia-jump-{pid}-{label}-{n}"));
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

// ─── Per-test helpers ───────────────────────────────────────────────────

fn lp_problem() -> Resource {
    // min x + 2y  s.t.  x + y ≤ 10,  0 ≤ x,y ≤ 10
    let objective = ft_add(ft_var("x"), ft_mul(ft_lit(2.0), ft_var("y")));
    let bounds = vec![
        build_variable_bound("x", Some(0.0), Some(10.0)),
        build_variable_bound("y", Some(0.0), Some(10.0)),
    ];
    let constraints = vec![build_constraint(
        ft_add(ft_var("x"), ft_var("y")),
        cr_le(),
        10.0,
    )];
    build_optimisation_problem(
        "lp_demo",
        &["x", "y"],
        bounds,
        objective,
        "Min",
        constraints,
    )
}

fn qp_problem() -> Resource {
    // min (x-1)² + (y-2)²  s.t.  x + y == 2
    let objective = ft_add(
        ft_pow(ft_sub(ft_var("x"), ft_lit(1.0)), ft_lit(2.0)),
        ft_pow(ft_sub(ft_var("y"), ft_lit(2.0)), ft_lit(2.0)),
    );
    let constraints = vec![build_constraint(
        ft_add(ft_var("x"), ft_var("y")),
        cr_eq(),
        2.0,
    )];
    build_optimisation_problem(
        "qp_demo",
        &["x", "y"],
        vec![],
        objective,
        "Min",
        constraints,
    )
}

fn float_property(r: &Resource, prop_iri: &str) -> f64 {
    r.get(&iri(prop_iri))
        .and_then(|v| match v {
            Value::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing or non-float property `{prop_iri}`"))
}

fn string_property(r: &Resource, prop_iri: &str) -> String {
    r.get(&iri(prop_iri))
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| panic!("missing or non-string property `{prop_iri}`"))
}

// ─── The probe ──────────────────────────────────────────────────────────

#[ignore = "heavy E2E: JuMP+HiGHS env image build (LP/QP round-trip)."]
#[test]
fn jump_highs_e2e_lp_and_qp_round_trip() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping jump-highs e2e: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping (could not pin base image): {e}");
            return;
        }
    };

    let storage = eigenius_kernel::layer::LayerStorage::in_memory();
    let head = std::sync::Arc::new(
        eigenius_kernel::layer::LayerBuilder::new("jump_highs_test", None).build(storage.clone()),
    );
    let exec_ctx = ExecutionContext::new(
        std::sync::Arc::clone(&head),
        "jump_highs_test",
        ExecutionMode::ReadOnly,
        storage,
    );

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
    let handler_pkg =
        build_handler_package("EigeniusJuMPHiGHS", HANDLER_PROJECT_TOML, HANDLER_SOURCE_JL);
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
    let dispatcher = Arc::new(Mutex::new(dispatcher));

    {
        let setup = "begin; using EigeniusJuMPHiGHS; nothing; end";
        let setup_arg = build_setup_argument(setup);
        let dispatcher = dispatcher.lock().expect("dispatcher mutex");
        if let Err(e) = dispatcher.dispatch_run_runtime_script(&[], &setup_arg) {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("setup script (using EigeniusJuMPHiGHS) failed: {e:?}");
        }
    }

    // Two institution wrappers — one per signature — sharing the dispatcher.
    let solve_inst = SubstrateBackedInstitution {
        institution_iri: iri(JUMP_HIGHS_INST_IRI),
        handler_method: "solve_problem".to_string(),
        handler_signature: SOLVE_PROBLEM_SIG_IRI.to_string(),
        env_iri: ENV_IRI.to_string(),
        image_digest: digest.as_str().to_string(),
        dispatcher: dispatcher.clone(),
    };
    let validate_inst = SubstrateBackedInstitution {
        institution_iri: iri("urn:eigenius:institutions:jump_highs_validate"),
        handler_method: "validate_optimum".to_string(),
        handler_signature: VALIDATE_OPTIMUM_SIG_IRI.to_string(),
        env_iri: ENV_IRI.to_string(),
        image_digest: digest.as_str().to_string(),
        dispatcher: dispatcher.clone(),
    };
    let mut inst_runtime = InstitutionRuntime::new();
    inst_runtime
        .register(Box::new(solve_inst))
        .expect("register solve");
    inst_runtime
        .register(Box::new(validate_inst))
        .expect("register validate");

    let solve = inst_runtime
        .get(&iri(JUMP_HIGHS_INST_IRI))
        .expect("solve institution registered");
    let validate = inst_runtime
        .get(&iri("urn:eigenius:institutions:jump_highs_validate"))
        .expect("validate institution registered");

    // ─── Test 1: LP ─────────────────────────────────────────────────────
    let lp = lp_problem();
    let lp_solve = match solve.query(&iri(SOLVE_PROBLEM_SIG_IRI), &lp, &exec_ctx) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("LP solve_problem dispatch failed: {e}");
        }
    };
    let lp_optimum = lp_solve.output;
    let lp_status = string_property(&lp_optimum, "urn:eigenius:jump:termination_status");
    let lp_obj = float_property(&lp_optimum, "urn:eigenius:jump:objective_value");
    assert_eq!(lp_status, "OPTIMAL", "LP termination status");
    assert!(lp_obj.abs() < 1e-6, "LP optimum should be 0; got {lp_obj}");

    // Build a re-claim from the solver's output and dispatch validate.
    let lp_claim =
        build_optimises_to_claim(lp_problem(), "OPTIMAL", lp_obj, &[0.0, 0.0], 1e-6, 1e-6);
    let lp_verdict = match validate.query(&iri(VALIDATE_OPTIMUM_SIG_IRI), &lp_claim, &exec_ctx) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("LP validate_optimum dispatch failed: {e}");
        }
    };
    let lp_ctor = string_property(&lp_verdict.output, "urn:eigenius:core:ctor_name");
    assert_eq!(lp_ctor, "Holds", "LP re-validation should Hold");

    // ─── Test 2: QP (smart-pow) ─────────────────────────────────────────
    let qp = qp_problem();
    let qp_solve = match solve.query(&iri(SOLVE_PROBLEM_SIG_IRI), &qp, &exec_ctx) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("QP solve_problem dispatch failed: {e}");
        }
    };
    let qp_optimum = qp_solve.output;
    let qp_status = string_property(&qp_optimum, "urn:eigenius:jump:termination_status");
    let qp_obj = float_property(&qp_optimum, "urn:eigenius:jump:objective_value");
    assert_eq!(qp_status, "OPTIMAL", "QP termination status");
    assert!(
        (qp_obj - 0.5).abs() < 1e-4,
        "QP optimum should be 0.5; got {qp_obj}"
    );

    let qp_claim =
        build_optimises_to_claim(qp_problem(), "OPTIMAL", qp_obj, &[0.5, 1.5], 1e-4, 1e-4);
    let qp_verdict = match validate.query(&iri(VALIDATE_OPTIMUM_SIG_IRI), &qp_claim, &exec_ctx) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("QP validate_optimum dispatch failed: {e}");
        }
    };
    let qp_ctor = string_property(&qp_verdict.output, "urn:eigenius:core:ctor_name");
    assert_eq!(qp_ctor, "Holds", "QP re-validation should Hold");

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}
