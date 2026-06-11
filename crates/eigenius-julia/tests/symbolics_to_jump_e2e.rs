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

//! Phase 19f.1 / D27 §4.2 — Symbolics → JuMP comorphism e2e.
//!
//! Worked example: fit the inhibition constant `Ki` of a competitive
//! inhibitor from observed IC50 measurements at known substrate
//! concentrations. The closed-form competitive-inhibition relation is
//!
//!     IC50 = Ki * (1 + [S] / Km)
//!
//! With Km known (assumed = 10) and observations
//!
//!     [S]      = [10, 50, 100]
//!     IC50_obs = [4,  12,  22]
//!
//! the linear coefficients on Ki are c_i = (1 + S_i / Km) = [2, 6, 11];
//! the SSE objective `Σ (IC50_obs_i − Ki·c_i)²` is exactly minimised at
//! `Ki* = 2.0` with `SSE* = 0`. (The observations were chosen
//! deliberately consistent with `Ki = 2`, so the test verifies the
//! comorphism + solver pipeline rather than the noise tolerance of
//! the fit.)
//!
//! The test runs the full FormulaTerm-everywhere pipeline:
//!
//! 1. Author the SSE as a `SymbolicExpression(term: FormulaTerm)`.
//! 2. Wrap it in a `SymbolicsToJuMPInput` carrying the JuMP-side
//!    framing (variable_names = ["Ki"], variable_bounds, sense, no
//!    constraints).
//! 3. Dispatch Symbolics' OnDemand `qc_symb_to_jump` (handler:
//!    `frame_as_optimisation_problem`) — the operational backing of
//!    the Symbolics → JuMP comorphism. The handler reads
//!    `objective.term` (identity on FormulaTerm) and packs an
//!    `OptimisationProblem`.
//! 4. Dispatch JuMP-HiGHS' OnDemand `qc_jump_solve` on the produced
//!    OptimisationProblem. The walker's smart-pow rule unrolls each
//!    `pow(•, LitFloat(2.0))` to `(• * •)` so HiGHS sees `QuadExpr`.
//!    Returns an `OptimisesTo` with `objective_value ≈ 0` and
//!    `variable_values ≈ [2.0]`.
//! 5. Dispatch JuMP-HiGHS' `validate_optimum` on the OptimisesTo.
//!    Asserts Holds.
//!
//! Single env image with both EigeniusSymbolics + EigeniusJuMPHiGHS
//! handler packages baked in; same Julia worker dispatches all three
//! signatures. Skipped on hosts without buildah / Docker / a pullable
//! Julia base image. Cold env build is moderate (~5–8 min — Symbolics
//! + JuMP + HiGHS plus the SymbolicUtils + MathOptInterface tail).

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
const SYMBOLICS_ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/symbolics/declarations/symbolics-ontology.eigon.json"
);
const FORMULAS_ONTOLOGY_JSON: &str =
    include_str!("../../../ontologies/formulas/formulas-ontology.json");

const SYMBOLICS_HANDLER_PROJECT_TOML: &str =
    include_str!("../../../julia/institutions/symbolics/EigeniusSymbolics/Project.toml");
const SYMBOLICS_HANDLER_SOURCE_JL: &str = include_str!(
    "../../../julia/institutions/symbolics/EigeniusSymbolics/src/EigeniusSymbolics.jl"
);
const JUMP_HANDLER_PROJECT_TOML: &str =
    include_str!("../../../julia/institutions/jump/EigeniusJuMPHiGHS/Project.toml");
const JUMP_HANDLER_SOURCE_JL: &str =
    include_str!("../../../julia/institutions/jump/EigeniusJuMPHiGHS/src/EigeniusJuMPHiGHS.jl");

// ─── IRIs the test pins ─────────────────────────────────────────────────

const SYMBOLICS_INST_IRI: &str = "urn:eigenius:institutions:symbolics";
const JUMP_HIGHS_INST_IRI: &str = "urn:eigenius:institutions:jump_highs";
const JUMP_HIGHS_VALIDATE_INST_IRI: &str = "urn:eigenius:institutions:jump_highs_validate";

const FRAME_AS_OPTIMISATION_SIG_IRI: &str =
    "urn:eigenius:symbolics:signatures:frame_as_optimisation_problem";
const SOLVE_PROBLEM_SIG_IRI: &str = "urn:eigenius:jump_highs:signatures:solve_problem";
const VALIDATE_OPTIMUM_SIG_IRI: &str = "urn:eigenius:jump_highs:signatures:validate_optimum";

const SYMBOLICS_TO_JUMP_INPUT_CLASS: &str = "urn:eigenius:symbolics:SymbolicsToJuMPInput";
const SYMBOLIC_EXPRESSION_CLASS: &str = "urn:eigenius:symbolics:SymbolicExpression";
const SIMPLIFIES_TO_CLASS: &str = "urn:eigenius:symbolics:SimplifiesTo";
const OPTIMISATION_PROBLEM_CLASS: &str = "urn:eigenius:jump:OptimisationProblem";
const OPTIMISES_TO_CLASS: &str = "urn:eigenius:jump:OptimisesTo";
const VARIABLE_BOUND_CLASS: &str = "urn:eigenius:jump:VariableBound";

const ENV_IRI: &str = "urn:eigenius:test:symb_to_jump:env";
const BASE_IMAGE_TAG: &str = "julia:1.12-bookworm";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

// ─── FormulaTerm builders ───────────────────────────────────────────────

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

// ─── Kinase Ki-fit objective ────────────────────────────────────────────
//
// SSE(Ki) = Σ_i (IC50_obs_i − Ki·c_i)²
//
// One residual term: `pow(sub(IC50_obs_i, mul(c_i, Ki)), 2.0)`
// SSE: residuals chained with `add`.
//
// Coefficients c_i = 1 + S_i / Km with Km = 10:
//   S = [10, 50, 100]
//   c = [2.0, 6.0, 11.0]
// True Ki* = 2.0, exact-fit observations:
//   IC50_obs = [4.0, 12.0, 22.0]

fn residual_term(ic50_obs: f64, coef: f64) -> serde_json::Value {
    ft_pow(
        ft_sub(ft_lit(ic50_obs), ft_mul(ft_lit(coef), ft_var("Ki"))),
        ft_lit(2.0),
    )
}

fn sse_objective_formula() -> serde_json::Value {
    let r1 = residual_term(4.0, 2.0);
    let r2 = residual_term(12.0, 6.0);
    let r3 = residual_term(22.0, 11.0);
    ft_add(ft_add(r1, r2), r3)
}

// ─── Resource builders ──────────────────────────────────────────────────

fn build_symbolic_expression() -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(SYMBOLIC_EXPRESSION_CLASS))]),
    );
    r.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("ki_fit_sse".into()),
    );
    r.set(
        iri("urn:eigenius:symbolics:term"),
        Value::Json(sse_objective_formula()),
    );
    r
}

fn build_variable_bound(name: &str, lower: f64, upper: f64) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(VARIABLE_BOUND_CLASS))]),
    );
    r.set(
        iri("urn:eigenius:jump:variable_name"),
        Value::String(name.to_string()),
    );
    r.set(iri("urn:eigenius:jump:lower"), Value::Float(lower));
    r.set(iri("urn:eigenius:jump:upper"), Value::Float(upper));
    r
}

fn build_symbolics_to_jump_input_cbor() -> Vec<u8> {
    let objective = build_symbolic_expression();
    let bound = build_variable_bound("Ki", 0.0, 10.0);

    let mut req = Resource::new_embedded();
    req.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(SYMBOLICS_TO_JUMP_INPUT_CLASS))]),
    );
    req.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("ki_fit_input".into()),
    );
    req.set(
        iri("urn:eigenius:symbolics:objective"),
        Value::Embedded(Box::new(objective)),
    );
    req.set(
        iri("urn:eigenius:symbolics:variable_names"),
        Value::Array(vec![Value::String("Ki".into())]),
    );
    req.set(
        iri("urn:eigenius:symbolics:framing_variable_bounds"),
        Value::Array(vec![Value::Embedded(Box::new(bound))]),
    );
    req.set(
        iri("urn:eigenius:symbolics:sense"),
        Value::String("Min".into()),
    );
    // `framing_constraints` is recommended; the kinase Ki-fit problem
    // has no algebraic constraints (only the bound `Ki ∈ [0, 10]`),
    // so omit the field. The chain validator's empty-array rule
    // rejects `Value::Array(vec![])`, so omission rather than empty.
    eigon_cbor::serialize_resource(&req)
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
        for json in [
            JUMP_ONTOLOGY_JSON,
            SYMBOLICS_ONTOLOGY_JSON,
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

fn build_mirror() -> Resource {
    let g = JuliaMirrorGenerator::new();
    let chain = CrossChain::new();
    let layer_iri = iri("urn:eigenius:test:symb_to_jump:layer");
    // Seed the closure: SymbolicsToJuMPInput pulls in SymbolicExpression
    // (objective), VariableBound + Constraint + ConstraintRelation +
    // FormulaTerm + operator catalog (transitively); OptimisationProblem
    // + OptimisesTo come from JuMP's surface; SimplifiesTo is needed
    // because EigeniusSymbolics's `validate_simplifies_to` is ungated
    // (top-level method dispatch references SimplifiesTo unconditionally,
    // so the symbol must resolve in the mirror or the package fails to
    // precompile — same reason ConservationLaw is seeded in the
    // catalyst→diffeq e2e).
    let seed = vec![
        iri(SYMBOLICS_TO_JUMP_INPUT_CLASS),
        iri(OPTIMISATION_PROBLEM_CLASS),
        iri(OPTIMISES_TO_CLASS),
        iri(SIMPLIFIES_TO_CLASS),
    ];
    let out = g
        .generate(&MirrorGenerationRequest {
            source_layer: &layer_iri,
            seed_classes: &seed,
            chain: &chain,
        })
        .expect("cross-institution mirror generation");
    for required in [
        SYMBOLICS_TO_JUMP_INPUT_CLASS,
        OPTIMISATION_PROBLEM_CLASS,
        OPTIMISES_TO_CLASS,
        SIMPLIFIES_TO_CLASS,
        SYMBOLIC_EXPRESSION_CLASS,
        VARIABLE_BOUND_CLASS,
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
        "urn:eigenius:test:symb_to_jump:handler-package:{name}"
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
    let dir = std::env::temp_dir().join(format!("substrate-julia-symb2jump-{pid}-{label}-{n}"));
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

#[ignore = "heavy E2E: Symbolics + JuMP env image builds."]
#[test]
fn symbolics_to_jump_e2e_via_kinase_ki_fit() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping symbolics→jump e2e: {reason}");
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
        eigenius_kernel::layer::LayerBuilder::new("symb2jump_test", None).build(storage.clone()),
    );
    let exec_ctx = ExecutionContext::new(
        std::sync::Arc::clone(&head),
        "symb2jump_test",
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
    let symb_pkg = build_handler_package(
        "EigeniusSymbolics",
        SYMBOLICS_HANDLER_PROJECT_TOML,
        SYMBOLICS_HANDLER_SOURCE_JL,
    );
    let jump_pkg = build_handler_package(
        "EigeniusJuMPHiGHS",
        JUMP_HANDLER_PROJECT_TOML,
        JUMP_HANDLER_SOURCE_JL,
    );
    let env = Resource::new_embedded();
    let runtime_for_dispatch: Box<dyn LanguageRuntime> = Box::new(runtime.clone());
    let digest = match runtime_for_dispatch.build_environment_image(
        &env,
        &[symb_pkg, jump_pkg],
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

    {
        let setup = "begin; using EigeniusSymbolics; using EigeniusJuMPHiGHS; nothing; end";
        let setup_arg = build_setup_argument(setup);
        let dispatcher = dispatcher.lock().expect("dispatcher mutex");
        if let Err(e) = dispatcher.dispatch_run_runtime_script(&[], &setup_arg) {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("setup script failed: {e:?}");
        }
    }

    // Three institution wrappers sharing the dispatcher: Symbolics
    // for the comorphism's frame_as_optimisation_problem, JuMP-HiGHS
    // for solve_problem, JuMP-HiGHS-validate for validate_optimum.
    let symb_inst = SubstrateBackedInstitution {
        institution_iri: iri(SYMBOLICS_INST_IRI),
        handler_method: "frame_as_optimisation_problem".to_string(),
        handler_signature: FRAME_AS_OPTIMISATION_SIG_IRI.to_string(),
        env_iri: ENV_IRI.to_string(),
        image_digest: digest.as_str().to_string(),
        dispatcher: dispatcher.clone(),
    };
    let solve_inst = SubstrateBackedInstitution {
        institution_iri: iri(JUMP_HIGHS_INST_IRI),
        handler_method: "solve_problem".to_string(),
        handler_signature: SOLVE_PROBLEM_SIG_IRI.to_string(),
        env_iri: ENV_IRI.to_string(),
        image_digest: digest.as_str().to_string(),
        dispatcher: dispatcher.clone(),
    };
    let validate_inst = SubstrateBackedInstitution {
        institution_iri: iri(JUMP_HIGHS_VALIDATE_INST_IRI),
        handler_method: "validate_optimum".to_string(),
        handler_signature: VALIDATE_OPTIMUM_SIG_IRI.to_string(),
        env_iri: ENV_IRI.to_string(),
        image_digest: digest.as_str().to_string(),
        dispatcher: dispatcher.clone(),
    };
    let mut inst_runtime = InstitutionRuntime::new();
    inst_runtime
        .register(Box::new(symb_inst))
        .expect("register symbolics");
    inst_runtime
        .register(Box::new(solve_inst))
        .expect("register solve");
    inst_runtime
        .register(Box::new(validate_inst))
        .expect("register validate");

    let symbolics = inst_runtime
        .get(&iri(SYMBOLICS_INST_IRI))
        .expect("symbolics registered");
    let solver = inst_runtime
        .get(&iri(JUMP_HIGHS_INST_IRI))
        .expect("solver registered");
    let validator = inst_runtime
        .get(&iri(JUMP_HIGHS_VALIDATE_INST_IRI))
        .expect("validator registered");

    // Step 1: Symbolics' qc_symb_to_jump → OptimisationProblem.
    let input_cbor = build_symbolics_to_jump_input_cbor();
    let input_resource =
        eigon_cbor::parse_resource_lenient(&input_cbor).expect("decode input CBOR");
    let frame_outcome = match symbolics.query(
        &iri(FRAME_AS_OPTIMISATION_SIG_IRI),
        &input_resource,
        &exec_ctx,
    ) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("frame_as_optimisation_problem dispatch failed: {e}");
        }
    };
    let opt_problem = frame_outcome.output;
    // Sanity-check the framed problem.
    let var_names = opt_problem
        .get(&iri("urn:eigenius:jump:variable_names"))
        .and_then(|v| match v {
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .expect("OptimisationProblem must carry variable_names");
    assert_eq!(var_names, vec!["Ki".to_string()], "variable_names");
    assert_eq!(
        string_property(&opt_problem, "urn:eigenius:jump:sense"),
        "Min",
        "sense"
    );

    // Step 2: JuMP-HiGHS' qc_jump_solve → OptimisesTo.
    let solve_outcome = match solver.query(&iri(SOLVE_PROBLEM_SIG_IRI), &opt_problem, &exec_ctx) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("solve_problem dispatch failed: {e}");
        }
    };
    let optimises_to = solve_outcome.output;
    let status = string_property(&optimises_to, "urn:eigenius:jump:termination_status");
    let obj_value = float_property(&optimises_to, "urn:eigenius:jump:objective_value");
    let var_values = optimises_to
        .get(&iri("urn:eigenius:jump:variable_values"))
        .and_then(|v| match v {
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|x| match x {
                        Value::Float(f) => Some(*f),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .expect("OptimisesTo must carry variable_values");
    assert_eq!(status, "OPTIMAL", "Ki-fit termination status");
    assert!(
        obj_value.abs() < 1e-4,
        "Ki-fit SSE should be ≈ 0 (exact-fit observations); got {obj_value}"
    );
    assert_eq!(var_values.len(), 1, "Ki-fit variable_values length");
    assert!(
        (var_values[0] - 2.0).abs() < 1e-4,
        "Ki-fit Ki* should be ≈ 2.0; got {}",
        var_values[0]
    );

    // Step 3: JuMP-HiGHS' validate_optimum on the OptimisesTo claim.
    let validate_outcome =
        match validator.query(&iri(VALIDATE_OPTIMUM_SIG_IRI), &optimises_to, &exec_ctx) {
            Ok(o) => o,
            Err(e) => {
                let _ = runtime.drain();
                let _ = std::fs::remove_dir_all(&depot);
                panic!("validate_optimum dispatch failed: {e}");
            }
        };
    let ctor = string_property(&validate_outcome.output, "urn:eigenius:core:ctor_name");
    assert_eq!(
        ctor, "Holds",
        "JuMP-HiGHS should re-validate the Ki-fit optimum"
    );

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}
