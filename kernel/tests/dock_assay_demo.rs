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

//! D14 §13.4 M8 — worked-example demo.
//!
//! Plumbing-only end-to-end test of the institution surface, using
//! the dock→assay scenario from D14 §5.1. One source institution
//! (Dock), one target institution (Assay), one comorphism
//! (`dock_to_assay`) with a real transformation Component middle
//! (`cm_arrhenius` — the Arrhenius approximation IC₅₀ ≈ exp(-ΔG/RT)),
//! plus two QueryClasses against the assay institution: a Decidable
//! `within_tolerance` predicate and an AutoOnLoad
//! `assay_prediction_validity` check fired on AssayPrediction Load.
//!
//! Each `#[test]` exercises one institution dispatch path:
//!
//! - [`comorphism_translates_dock_to_assay`] — `Exp::InstitutionInvoke`,
//!   four-step pipeline (D14 §9.3).
//! - [`decidable_query_class_holds_in_tolerance`] /
//!   [`decidable_query_class_fails_outside_tolerance`] —
//!   `Exp::NativeDecide` against a Decidable QueryClass (D14 §9.2).
//! - [`auto_on_load_fires_on_assay_prediction`] — Load-time dispatch
//!   for an AutoOnLoad QueryClass (D14 §9.1).
//!
//! This test wires the institutions and transformation as in-process
//! Rust impls so the demo is self-contained and hermetic.

use std::sync::Arc;
use std::sync::Mutex;

use eigenius_kernel::bootstrap;
use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::registry::InstitutionIndex;
use eigenius_kernel::institution::runtime::{Institution, InstitutionRuntime};
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::component::{BuiltinComponent, ComponentRegistry, ComponentResult};

// ─── Constants from the demo ontology ──────────────────────────────────

const DEMO_ONTOLOGY: &str = include_str!("../../ontologies/examples/dock-assay/dock-assay.json");

const DOCK_INST_IRI: &str = "urn:eigenius:demo:institutions:dock";
const ASSAY_INST_IRI: &str = "urn:eigenius:demo:institutions:assay";
const DOCKING_RESULT_CLASS: &str = "urn:eigenius:demo:institutions:DockingResult";
const ASSAY_PREDICTION_CLASS: &str = "urn:eigenius:demo:institutions:AssayPrediction";
const DELTA_G_PROP: &str = "urn:eigenius:demo:institutions:delta_g";
const IC50_PROP: &str = "urn:eigenius:demo:institutions:ic50";
const PREDICTED_IC50_PROP: &str = "urn:eigenius:demo:institutions:predicted_ic50";
const TARGET_IC50_PROP: &str = "urn:eigenius:demo:institutions:target_ic50";
const TOLERANCE_PROP: &str = "urn:eigenius:demo:institutions:tolerance";
const EXTRACT_DG_PROC: &str = "urn:eigenius:demo:institutions:proc:extract_dg";
const REIFY_IC50_PROC: &str = "urn:eigenius:demo:institutions:proc:reify_ic50";
const WITHIN_TOLERANCE_PROC: &str = "urn:eigenius:demo:institutions:proc:within_tolerance";
const CHECK_ASSAY_PREDICTION_PROC: &str =
    "urn:eigenius:demo:institutions:proc:check_assay_prediction";
const VALIDATE_PREDICTION_PROC: &str = "urn:eigenius:demo:institutions:proc:validate_prediction";
const CANDIDATE_PROP: &str = "urn:eigenius:demo:institutions:candidate";
const ARRHENIUS_COMPONENT_IRI: &str = "urn:eigenius:demo:institutions:cm_arrhenius";

const DEMO_LAYER_NAME: &str = "dock-assay-demo";

// Arrhenius constants (matching the lambda in cm_arrhenius).
// IC₅₀ (nM) ≈ exp(-ΔG / (R·T)) · 1e9, with R·T at 310 K in kcal/mol.
const RT_KCAL_PER_MOL: f64 = 0.616; // R·T at ~310 K
const IC50_SCALE_NM: f64 = 1.0e9;

fn arrhenius_ic50_nm(delta_g_kcal: f64) -> f64 {
    (-delta_g_kcal / RT_KCAL_PER_MOL).exp() * IC50_SCALE_NM
}

// ─── Helpers ───────────────────────────────────────────────────────────

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-formed IRI")
}

fn as_float(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Float(f) => Some(*f),
        Value::Integer(n) => Some(*n as f64),
        _ => None,
    }
}

fn first_float_property(resource: &Resource) -> Option<f64> {
    for v in resource.properties().values() {
        if let Some(f) = as_float(Some(v)) {
            return Some(f);
        }
    }
    None
}

fn float_payload_resource(value: f64) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(iri("urn:eigenius:core:value"), Value::Float(value));
    r
}

/// Build the demo layer on top of the bootstrap chain. Returns the
/// layer paired with its `LayerStorage` so callers can thread the
/// same storage into a derived `ExecutionContext`.
fn build_demo_layer() -> (Arc<Layer>, LayerStorage) {
    let ctx = bootstrap::bootstrap().expect("bootstrap kernel");
    let parent = Arc::clone(ctx.head());
    let mut builder = LayerBuilder::new(DEMO_LAYER_NAME, Some(parent));
    let resources = eigon_json::parse_document(DEMO_ONTOLOGY).expect("parse demo ontology");
    for r in resources {
        builder.add_resource(r).expect("add demo resource");
    }
    let storage = LayerStorage::in_memory();
    (Arc::new(builder.build(storage.clone())), storage)
}

/// Build the InstitutionIndex from the demo layer chain.
fn build_demo_index(layer: &Layer) -> Arc<InstitutionIndex> {
    let (idx, errors) = InstitutionIndex::from_layer(layer);
    assert!(errors.is_empty(), "demo ontology index errors: {errors:?}");
    Arc::new(idx)
}

/// Build the InstitutionRuntime registering Dock + Assay.
fn build_demo_runtime() -> Arc<InstitutionRuntime> {
    let mut runtime = InstitutionRuntime::new();
    runtime
        .register(Box::new(DockInstitution::new()))
        .expect("register Dock");
    runtime
        .register(Box::new(AssayInstitution::new()))
        .expect("register Assay");
    Arc::new(runtime)
}

/// Build the ComponentRegistry registering the Arrhenius transformation.
fn build_demo_components() -> Arc<ComponentRegistry> {
    let mut registry = ComponentRegistry::default();
    registry.register(
        ARRHENIUS_COMPONENT_IRI.to_string(),
        Box::new(ArrheniusComponent),
    );
    Arc::new(registry)
}

fn build_exec_ctx(layer: Arc<Layer>, storage: LayerStorage) -> ExecutionContext {
    ExecutionContext::new(layer, "dock-assay-demo", ExecutionMode::ReadOnly, storage)
}

// ─── Dock institution ──────────────────────────────────────────────────

struct DockInstitution {
    iri: Iri,
}

impl DockInstitution {
    fn new() -> Self {
        Self {
            iri: iri(DOCK_INST_IRI),
        }
    }
}

impl Institution for DockInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.iri
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        resource: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        if procedure_iri.as_str() != EXTRACT_DG_PROC {
            return Err(InstitutionError::UnknownType(format!(
                "dock institution does not implement procedure `{procedure_iri}`"
            )));
        }
        let delta_g = as_float(resource.get(&iri(DELTA_G_PROP))).ok_or_else(|| {
            InstitutionError::ComputationFailed(format!(
                "DockingResource is missing required `{DELTA_G_PROP}` (Float)"
            ))
        })?;
        Ok(Val::ResourceVal(Box::new(float_payload_resource(delta_g))))
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        _value: &Val,
        _ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "dock institution does not implement reify (`{procedure_iri}`)"
        )))
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        _input: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<eigenius_kernel::institution::runtime::QueryOutcome, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "dock institution does not implement query (`{procedure_iri}`)"
        )))
    }
}

// ─── Assay institution ─────────────────────────────────────────────────

struct AssayInstitution {
    iri: Iri,
}

impl AssayInstitution {
    fn new() -> Self {
        Self {
            iri: iri(ASSAY_INST_IRI),
        }
    }

    fn within_tolerance_verdict(input: &Resource) -> &'static str {
        // Decidable QueryClass dispatch (D14 §9.2): the kernel
        // populates the input class's typed required properties from
        // positional ESL args in `requires` declaration order
        // (Phase 19d.7). For `WithinToleranceInput` the kernel sets
        // `predicted_ic50`, `target_ic50`, `tolerance` from
        // `decide(predicted, target, tol)`. Each is set as a wrapper
        // resource (the kernel marshals `Val::ResourceVal` as
        // `Value::Embedded`), so dig through with
        // `first_float_property`.
        let extract = |prop_iri: &str| -> Option<f64> {
            match input.get(&iri(prop_iri))? {
                Value::Float(f) => Some(*f),
                Value::Integer(n) => Some(*n as f64),
                Value::Embedded(r) => first_float_property(r),
                _ => None,
            }
        };
        let predicted = extract(PREDICTED_IC50_PROP);
        let target = extract(TARGET_IC50_PROP);
        let tolerance = extract(TOLERANCE_PROP);
        match (predicted, target, tolerance) {
            (Some(p), Some(t), Some(tol)) if tol >= 0.0 => {
                if (p - t).abs() <= tol {
                    wk::VERDICT_HOLDS
                } else {
                    wk::VERDICT_FAILS
                }
            }
            _ => wk::VERDICT_UNDECIDABLE,
        }
    }

    /// AutoOnLoad check: an AssayPrediction must have a positive IC50.
    /// A non-positive value indicates either a bug in the comorphism's
    /// transformation or a malformed manual import — Fails forces the
    /// caller to surface it.
    fn assay_prediction_verdict(input: &Resource) -> &'static str {
        match as_float(input.get(&iri(IC50_PROP))) {
            Some(v) if v.is_finite() && v > 0.0 => wk::VERDICT_HOLDS,
            Some(_) => wk::VERDICT_FAILS,
            None => wk::VERDICT_UNDECIDABLE,
        }
    }

    fn verdict_resource(ctor: &str) -> Resource {
        let mut r = Resource::new_embedded();
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::VERDICT.to_string())]),
        );
        r.set(iri(wk::CTOR_NAME), Value::String(ctor.to_string()));
        r
    }
}

impl Institution for AssayInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.iri
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        _resource: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "assay institution does not implement extract_typed (`{procedure_iri}`)"
        )))
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        value: &Val,
        _ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        if procedure_iri.as_str() != REIFY_IC50_PROC {
            return Err(InstitutionError::UnknownType(format!(
                "assay institution does not implement procedure `{procedure_iri}`"
            )));
        }
        let payload = match value {
            Val::ResourceVal(r) => r.as_ref().clone(),
            other => {
                return Err(InstitutionError::ComputationFailed(format!(
                    "assay reify expected ResourceVal payload, got {other:?}"
                )))
            }
        };
        let ic50 = first_float_property(&payload).ok_or_else(|| {
            InstitutionError::ComputationFailed("assay reify: payload carries no Float".into())
        })?;
        let mut prediction = Resource::new_embedded();
        prediction.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(ASSAY_PREDICTION_CLASS.to_string())]),
        );
        prediction.set(iri(IC50_PROP), Value::Float(ic50));
        Ok(prediction)
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<eigenius_kernel::institution::runtime::QueryOutcome, InstitutionError> {
        let result = match procedure_iri.as_str() {
            WITHIN_TOLERANCE_PROC => {
                let ctor = Self::within_tolerance_verdict(input);
                Self::verdict_resource(ctor)
            }
            CHECK_ASSAY_PREDICTION_PROC => {
                let ctor = Self::assay_prediction_verdict(input);
                Self::verdict_resource(ctor)
            }
            VALIDATE_PREDICTION_PROC => {
                // OnDemand QueryClass: input carries `candidate` →
                // AssayPrediction. Read it out and reuse the same
                // ic50-validity verdict as the AutoOnLoad path.
                let candidate_val = input.get(&iri(CANDIDATE_PROP));
                let candidate = match candidate_val {
                    Some(Value::Embedded(r)) => r.as_ref(),
                    Some(other) => {
                        return Err(InstitutionError::ComputationFailed(format!(
                            "validate_prediction: `candidate` must be an Embedded resource, got {other:?}"
                        )));
                    }
                    None => {
                        return Err(InstitutionError::ComputationFailed(
                            "validate_prediction: input is missing `candidate`".into(),
                        ));
                    }
                };
                let ctor = Self::assay_prediction_verdict(candidate);
                Self::verdict_resource(ctor)
            }
            _ => {
                return Err(InstitutionError::UnknownType(format!(
                    "assay institution does not implement procedure `{procedure_iri}`"
                )))
            }
        };
        Ok(eigenius_kernel::institution::runtime::QueryOutcome::from_output(result))
    }
}

// ─── Arrhenius transformation Component ────────────────────────────────

/// Pure scalar transformation Float → Float implementing
/// `cm_arrhenius`. The middle of the dock_to_assay comorphism. Reads
/// the single Float property off the input resource (the wrapper
/// shape `extract_typed` returns), applies the Arrhenius
/// approximation, and emits the result back as the same single-Float
/// wrapper shape that `reify` consumes.
struct ArrheniusComponent;

impl BuiltinComponent for ArrheniusComponent {
    fn execute(
        &self,
        input: &Resource,
        _argument: Option<&Resource>,
        _layer: &Layer,
    ) -> Result<ComponentResult, String> {
        let delta_g = first_float_property(input).ok_or_else(|| {
            "cm_arrhenius: input wrapper resource carries no Float payload".to_string()
        })?;
        let ic50_nm = arrhenius_ic50_nm(delta_g);
        Ok(ComponentResult {
            output: float_payload_resource(ic50_nm),
            metrics: None,
        })
    }
}

// ─── 1. Comorphism: four-step pipeline (D14 §9.3) ──────────────────────

/// `Exp::InstitutionInvoke { comorphism, source }` runs:
///   extract_typed (dock) → cm_arrhenius (Component) → reify (assay).
/// The post-translation invariant fires `assay_prediction_validity`
/// AutoOnLoad on the produced AssayPrediction; for in-tolerance ΔG
/// the resulting IC₅₀ is positive so the invariant Holds.
#[test]
fn comorphism_translates_dock_to_assay() {
    let (layer, _storage) = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime();
    let components = build_demo_components();

    let source = "
        namespace demo = \"urn:eigenius:demo:institutions\";

        program demo:translate : demo:DockingResult -> demo:AssayPrediction {
            demo:dock_to_assay(input)
        }
    ";

    let user_resources =
        eigenius_kernel::esl::compile_with_institutions(source, Arc::clone(&index))
            .expect("ESL compile");
    let mut user_builder = LayerBuilder::new("dock-assay-demo-program", Some(Arc::clone(&layer)));
    for r in user_resources {
        user_builder.add_resource(r).expect("add user resource");
    }
    let program_layer = Arc::new(user_builder.build(LayerStorage::in_memory()));

    // Build a sample DockingResult: ΔG = -8.5 kcal/mol.
    let mut input = Resource::new(iri("urn:eigenius:demo:institutions:input1"));
    input.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(DOCKING_RESULT_CLASS.to_string())]),
    );
    input.set(iri(DELTA_G_PROP), Value::Float(-8.5));

    let prog_iri = iri("urn:eigenius:demo:institutions:translate");
    let program = program_layer
        .resolve(&prog_iri)
        .expect("translate program in layer")
        .clone();

    let result = eigenius_kernel::program::eval_io::execute_program_nbe_with_institutions(
        &program,
        &input,
        Arc::clone(&program_layer),
        components,
        Some(index),
        Some(runtime),
        None,
        None,
    )
    .expect("comorphism dispatch");

    let ic50 = as_float(result.output.get(&iri(IC50_PROP))).expect("AssayPrediction.ic50");
    let expected = arrhenius_ic50_nm(-8.5);
    assert!(
        (ic50 - expected).abs() < expected * 1e-9,
        "expected IC50≈{expected}, got {ic50}"
    );

    let is_a = result.output.is_a();
    assert!(
        is_a.iter().any(|i| i.as_str() == ASSAY_PREDICTION_CLASS),
        "translated resource should be an AssayPrediction; got is_a={is_a:?}"
    );

    // D14 §9.3 step 4: the output Resource itself (which the
    // RunProgram RPC serializes into the response payload) must
    // carry the deterministic @id so clients can resolve it
    // against the chain.
    let output_iri = result
        .output
        .id()
        .expect("output Resource carries a chain-resident @id");
    assert!(
        output_iri
            .as_str()
            .starts_with("urn:eigenius:comorphism-output:dock_to_assay:"),
        "expected output @id under comorphism-output: namespace, got {output_iri}"
    );

    // The eval_traced wrapper produces a `Trace::Comorphism` node
    // for `Exp::InstitutionInvoke`. Without it, comorphism-only
    // programs would have `root_trace = None` and the run-boundary's
    // ProgramTrace commit would fail validation on missing
    // `trace_tree`. The trace records the dispatched comorphism IRI
    // and the produced resource's chain IRI + class — the structural
    // audit anchor for "this program ran this comorphism".
    let root_trace = result
        .root_trace
        .as_ref()
        .expect("comorphism dispatch produces a non-empty root trace");
    match root_trace {
        eigenius_kernel::program::trace::Trace::Comorphism {
            comorphism_iri,
            target_iri: trace_target_iri,
            target_class,
            ..
        } => {
            assert_eq!(
                comorphism_iri, "urn:eigenius:demo:institutions:dock_to_assay",
                "trace records the dispatched comorphism IRI"
            );
            assert_eq!(
                trace_target_iri,
                output_iri.as_str(),
                "trace's target_iri matches the output Resource @id"
            );
            assert_eq!(
                target_class, ASSAY_PREDICTION_CLASS,
                "trace records the produced resource's class"
            );
        }
        other => panic!("expected Trace::Comorphism root, got {other:?}"),
    }

    // D14 §9.3 step 4: the reified target-class resource must enter
    // the chain. The reify boundary stamps a deterministic
    // `urn:eigenius:comorphism-output:<tail>:<hex>` IRI and pushes
    // the resource into the run-boundary collector; here we assert
    // the collector saw it.
    assert_eq!(
        result.produced_resources.len(),
        1,
        "expected exactly one produced resource (the reified AssayPrediction)"
    );
    let produced = &result.produced_resources[0];
    let produced_iri = produced
        .id()
        .expect("produced resource has chain-resident @id");
    assert!(
        produced_iri
            .as_str()
            .starts_with("urn:eigenius:comorphism-output:dock_to_assay:"),
        "expected deterministic comorphism-output IRI, got {produced_iri}"
    );
    let produced_ic50 =
        as_float(produced.get(&iri(IC50_PROP))).expect("produced AssayPrediction.ic50");
    assert!(
        (produced_ic50 - expected).abs() < expected * 1e-9,
        "produced resource should carry the same IC50 payload"
    );

    // Determinism: re-running with identical input produces the
    // identical content-hash IRI. This is the chain-dedup property
    // that makes "two paths arriving at the same sentence" land at
    // the same resource.
    let result2 = eigenius_kernel::program::eval_io::execute_program_nbe_with_institutions(
        &program,
        &input,
        Arc::clone(&program_layer),
        build_demo_components(),
        Some(build_demo_index(&program_layer)),
        Some(build_demo_runtime()),
        None,
        None,
    )
    .expect("second comorphism dispatch");
    let produced2_iri = result2.produced_resources[0]
        .id()
        .expect("re-run produced resource has @id");
    assert_eq!(
        produced_iri, produced2_iri,
        "re-running with identical input must mint the same deterministic IRI"
    );
}

// ─── 2. Decidable QueryClass dispatch (D14 §9.2) ───────────────────────

/// Build a `Constraint::Institution` that calls `within_tolerance` with
/// three Float arguments. Returns the program's eval result (Refl on
/// Holds, neutral on Fails / Undecidable).
fn run_within_tolerance(predicted: f64, target: f64, tolerance: f64) -> Val {
    use eigenius_kernel::nbe::env::Rho;
    use eigenius_kernel::nbe::eval::{eval_ctx, EvalCtx};
    use eigenius_kernel::nbe::term::{Constraint, Exp, PrimitiveType};

    let (layer, _storage) = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime();
    let components = build_demo_components();
    let dispatched_traces = Arc::new(Mutex::new(Vec::new()));

    let engine = eigenius_kernel::institution::eval_hooks::InstitutionEngine::for_io(
        Arc::clone(&layer),
        components,
        None,
        dispatched_traces,
        Arc::new(Mutex::new(Vec::new())),
        None,
        Some(index),
        Some(runtime),
    );
    let ctx = EvalCtx::effectful(Some(Arc::clone(&layer)), Arc::new(engine));

    let wrap_float = |f: f64| -> Exp {
        let mut r = Resource::new_embedded();
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:eigenius:core:Float".to_string())]),
        );
        r.set(iri("urn:eigenius:core:value"), Value::Float(f));
        Exp::EigonResource(Box::new(r))
    };
    let _ = PrimitiveType::Float; // silence unused-import

    // Construct: NativeDecide(Constraint::Institution { iri = within_tolerance, args = [predicted, target, tolerance] }, Unit).
    let constraint = Constraint::Institution {
        iri: iri("urn:eigenius:demo:institutions:within_tolerance"),
        args: vec![
            wrap_float(predicted),
            wrap_float(target),
            wrap_float(tolerance),
        ],
    };
    let exp = Exp::NativeDecide(constraint, Box::new(Exp::Unit));

    eval_ctx(&exp, &Rho::Nil, &ctx).expect("decide eval")
}

#[test]
fn decidable_query_class_holds_in_tolerance() {
    use eigenius_kernel::nbe::val::Val;
    // |500 - 600| = 100 ≤ 200 tolerance → Holds → eval folds NativeDecide to Refl.
    let v = run_within_tolerance(500.0, 600.0, 200.0);
    assert!(matches!(v, Val::Refl(_)), "expected Refl(Unit), got {v:?}");
}

#[test]
fn decidable_query_class_fails_outside_tolerance() {
    use eigenius_kernel::nbe::eval::EvalError;
    use eigenius_kernel::nbe::val::{Neut, Val};
    // |500 - 600| = 100 > 50 tolerance → Fails → eval emits a failing neutral.
    let _ = EvalError::ModeError(String::new()); // silence unused-import on small surface
    let v = run_within_tolerance(500.0, 600.0, 50.0);
    match v {
        Val::Nt(Neut::Gen(_, name)) => {
            assert_eq!(name, "__constraint_failed");
        }
        other => panic!("expected failing neutral, got {other:?}"),
    }
}

// ─── 3. AutoOnLoad QueryClass dispatch (D14 §9.1) ──────────────────────

/// `assay_prediction_validity` is bound AutoOnLoad to AssayPrediction.
/// A positive-IC₅₀ instance Holds; a non-positive instance Fails.
#[test]
fn auto_on_load_fires_on_assay_prediction() {
    use eigenius_kernel::institution::dispatch::dispatch_auto_on_load_for_resource;

    let (layer, storage) = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime();
    let exec_ctx = build_exec_ctx(Arc::clone(&layer), storage);

    // Healthy AssayPrediction — IC₅₀ = 250 nM.
    let mut good = Resource::new(iri("urn:eigenius:demo:institutions:good_prediction"));
    good.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(ASSAY_PREDICTION_CLASS.to_string())]),
    );
    good.set(iri(IC50_PROP), Value::Float(250.0));
    let errs =
        dispatch_auto_on_load_for_resource(&good, &index, &runtime, &exec_ctx).flatten_to_errors();
    assert!(
        errs.is_empty(),
        "Holds should produce no AutoOnLoad errors; got {errs:?}"
    );

    // Broken AssayPrediction — non-positive IC₅₀ should Fail the
    // AutoOnLoad check, surfacing as a typed ValidationError.
    let mut bad = Resource::new(iri("urn:eigenius:demo:institutions:bad_prediction"));
    bad.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(ASSAY_PREDICTION_CLASS.to_string())]),
    );
    bad.set(iri(IC50_PROP), Value::Float(-1.0));
    let errs =
        dispatch_auto_on_load_for_resource(&bad, &index, &runtime, &exec_ctx).flatten_to_errors();
    assert_eq!(errs.len(), 1, "expected one Fails error; got {errs:?}");
    assert!(
        errs[0].message.contains("returned Fails"),
        "unexpected message: {}",
        errs[0].message
    );
}

// ─── 4. FIBER param comorphism coercion (D2 v2 §3.5 / §6.12) ───────────

/// FIBER param coercion: `param: dock_to_assay(?d)` runs the four-step
/// pipeline inline as part of FIBER input marshalling. The dispatch
/// path mirrors `Exp::InstitutionInvoke` — same comorphism, same
/// transformation Component, same post-translation invariant — but
/// reached via the EigenQL evaluator's helper. Here we drive the
/// helper directly with the dock-assay layer/index/runtime/components
/// to verify the EigenQL-side wiring works end-to-end.
#[test]
fn fiber_param_comorphism_coercion_runs_four_step_pipeline() {
    use eigenius_kernel::ontology::resource::Resource;
    use eigenius_kernel::query::ast::{Expression, Name};
    use std::collections::BTreeMap;

    let (layer, storage) = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime();
    let components = build_demo_components();
    let exec_ctx = build_exec_ctx(Arc::clone(&layer), storage);

    // Build a sample DockingResult and bind it to ?d in the binding.
    let dock_iri = iri("urn:eigenius:demo:institutions:input1");
    let mut docking = Resource::new(dock_iri.clone());
    docking.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(DOCKING_RESULT_CLASS.to_string())]),
    );
    docking.set(iri(DELTA_G_PROP), Value::Float(-8.5));
    let mut binding: BTreeMap<String, Value> = BTreeMap::new();
    binding.insert("d".into(), Value::Embedded(Box::new(docking)));

    let comorphism_name = Name::FullIri(iri("urn:eigenius:demo:institutions:dock_to_assay"));
    let source_expr =
        Expression::Variable(eigenius_kernel::query::ast::Variable { name: "d".into() });

    let reified = eigenius_kernel::query::evaluate::eval_comorphism_coercion(
        &comorphism_name,
        &source_expr,
        &binding,
        &layer,
        &index,
        &runtime,
        &components,
        &exec_ctx,
    )
    .expect("coercion four-step pipeline");

    let resource = match reified {
        Value::Embedded(r) => *r,
        other => panic!("expected Embedded AssayPrediction, got {other:?}"),
    };
    assert!(
        resource
            .is_a()
            .iter()
            .any(|i| i.as_str() == ASSAY_PREDICTION_CLASS),
        "expected reified resource to be an AssayPrediction; got is_a={:?}",
        resource.is_a()
    );
    let ic50 = as_float(resource.get(&iri(IC50_PROP))).expect("AssayPrediction.ic50");
    let expected = arrhenius_ic50_nm(-8.5);
    assert!(
        (ic50 - expected).abs() < expected * 1e-9,
        "expected IC50≈{expected}, got {ic50}"
    );
}

// ─── 5. EigenQL queries — full surface against the demo (D2 v2) ────────

/// Build a data layer with a sample DockingResult resource the
/// EigenQL queries can MATCH against.
fn build_demo_data_layer() -> (Arc<Layer>, LayerStorage) {
    let (demo, _) = build_demo_layer();
    let mut builder = LayerBuilder::new("dock-assay-demo-data", Some(demo));
    let mut docking = Resource::new(iri("urn:eigenius:demo:institutions:dock-result-1"));
    docking.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(DOCKING_RESULT_CLASS.to_string())]),
    );
    docking.set(iri(DELTA_G_PROP), Value::Float(-8.5));
    builder.add_resource(docking).expect("add docking resource");
    let storage = LayerStorage::in_memory();
    (Arc::new(builder.build(storage.clone())), storage)
}

/// Sanity check: the demo data layer's DockingResult is matchable
/// via EigenQL MATCH. If this fails, the FIBER tests below are
/// vacuously zero-row.
#[test]
fn eigenql_match_finds_demo_docking_result() {
    use eigenius_kernel::query;
    let (data, _storage) = build_demo_data_layer();
    let runtime = query::evaluate::FiberRuntime::default();
    let source = r#"
        MATCH "urn:eigenius:demo:institutions:DockingResult"(?d) {
            "urn:eigenius:demo:institutions:delta_g": ?dg
        }
        RETURN [] { d: ?d }
    "#;
    let document = query::execute_with(source, &data, runtime).expect("query executes");
    let result_set = document
        .iter()
        .find(|r| {
            r.is_a()
                .iter()
                .any(|i| i.as_str() == "urn:eigenius:query:ResultSet")
        })
        .expect("ResultSet");
    let row_count = match result_set.get(&iri("urn:eigenius:query:row_count")) {
        Some(Value::Integer(n)) => *n,
        _ => panic!("ResultSet missing row_count"),
    };
    assert_eq!(row_count, 1, "MATCH should find the demo DockingResult");
}

/// Diagnostic: FIBER + comorphism coercion alone (no postfix). Should
/// produce one row with ?v bound to the Verdict resource IRI.
#[test]
fn eigenql_fiber_coercion_only_produces_verdict_binding() {
    use eigenius_kernel::query;
    let (data, storage) = build_demo_data_layer();
    let index = build_demo_index(&data);
    let inst_runtime = build_demo_runtime();
    let components = build_demo_components();
    let exec_ctx = build_exec_ctx(Arc::clone(&data), storage);
    let runtime = query::evaluate::FiberRuntime {
        index: Some(&index),
        runtime: Some(&inst_runtime),
        components: Some(&components),
        overlay: None,
        ctx: Some(&exec_ctx),
        similarity: None,
        embedders: None,
        embedding_cache: None,
        vector_segment_cache: None,
    };
    let source = r#"
        USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
        USING NAMESPACE "urn:eigenius:demo:institutions:"

        MATCH "urn:eigenius:demo:institutions:DockingResult"(?d) {
            "urn:eigenius:demo:institutions:delta_g": ?dg
        }
        FIBER assay:validate_prediction {
            candidate: "urn:eigenius:demo:institutions:dock_to_assay"(?d)
        } AS ?v
        RETURN [] { d: ?d, v: ?v }
    "#;
    let document = query::execute_with(source, &data, runtime).expect("query executes");
    let result_set = document
        .iter()
        .find(|r| {
            r.is_a()
                .iter()
                .any(|i| i.as_str() == "urn:eigenius:query:ResultSet")
        })
        .expect("ResultSet");
    let row_count = match result_set.get(&iri("urn:eigenius:query:row_count")) {
        Some(Value::Integer(n)) => *n,
        _ => panic!("ResultSet missing row_count"),
    };
    assert_eq!(row_count, 1, "FIBER should produce exactly one row");
}

/// EigenQL FIBER + comorphism coercion + postfix HOLDS — the canonical
/// D2 v2 §3.5 / §3.8 surface composed end-to-end. The query:
///
/// 1. MATCHes a `DockingResult` in the data layer.
/// 2. FIBER-dispatches `validate_prediction` against the assay
///    institution, with `candidate` set via comorphism coercion of
///    the matched DockingResult through `dock_to_assay`.
/// 3. Filters on `?v HOLDS` (the postfix Verdict predicate projects
///    the ?v Verdict resource to a Boolean).
/// 4. RETURNs the matched DockingResult IRI.
#[test]
fn eigenql_fiber_with_comorphism_coercion_and_postfix_holds() {
    use eigenius_kernel::query;

    let (data, storage) = build_demo_data_layer();
    let index = build_demo_index(&data);
    let runtime = build_demo_runtime();
    let components = build_demo_components();
    let exec_ctx = build_exec_ctx(Arc::clone(&data), storage);

    let runtime = query::evaluate::FiberRuntime {
        index: Some(&index),
        runtime: Some(&runtime),
        components: Some(&components),
        overlay: None,
        ctx: Some(&exec_ctx),
        similarity: None,
        embedders: None,
        embedding_cache: None,
        vector_segment_cache: None,
    };

    let source = r#"
        USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
        USING NAMESPACE "urn:eigenius:demo:institutions:"

        MATCH "urn:eigenius:demo:institutions:DockingResult"(?d) {
            "urn:eigenius:demo:institutions:delta_g": ?dg
        }
        FIBER assay:validate_prediction {
            candidate: "urn:eigenius:demo:institutions:dock_to_assay"(?d)
        } AS ?v
        WHERE ?v HOLDS
        RETURN [] {
            d: ?d
        }
    "#;

    let document = query::execute_with(source, &data, runtime).expect("query executes");

    // Expect exactly one row — the matched DockingResult survived the
    // postfix-HOLDS filter (Arrhenius IC₅₀ for ΔG=-8.5 is positive,
    // so validate_prediction returns Holds).
    let result_set = document
        .iter()
        .find(|r| {
            r.is_a()
                .iter()
                .any(|i| i.as_str() == "urn:eigenius:query:ResultSet")
        })
        .expect("ResultSet in document");
    let row_count = match result_set.get(&iri("urn:eigenius:query:row_count")) {
        Some(Value::Integer(n)) => *n,
        _ => panic!("ResultSet missing row_count"),
    };
    assert_eq!(row_count, 1, "expected one row, got {row_count}");
}

/// EigenQL FIBER + INTO (D14 §9.3 chain-reinsertion via EigenQL —
/// Phase 19i Phase 2). With `INTO "<iri>"`, the FIBER's response
/// resource is committed to the regular chain at the named IRI as
/// part of the query's outcome rather than disappearing with the
/// per-query overlay. The QueryOutcome carries the to-be-committed
/// resources so the server's Query RPC can lift them through the
/// commit orchestrator (D41 §10).
#[test]
fn eigenql_fiber_into_collects_response_for_chain_commit() {
    use eigenius_kernel::query;

    let (data, storage) = build_demo_data_layer();
    let index = build_demo_index(&data);
    let runtime_inst = build_demo_runtime();
    let components = build_demo_components();
    let exec_ctx = build_exec_ctx(Arc::clone(&data), storage);

    let runtime = query::evaluate::FiberRuntime {
        index: Some(&index),
        runtime: Some(&runtime_inst),
        components: Some(&components),
        overlay: None,
        ctx: Some(&exec_ctx),
        similarity: None,
        embedders: None,
        embedding_cache: None,
        vector_segment_cache: None,
    };

    let target = "urn:eigenius:demo:institutions:my_validation_verdict";
    let source = format!(
        r#"
        USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
        USING NAMESPACE "urn:eigenius:demo:institutions:"

        MATCH "urn:eigenius:demo:institutions:DockingResult"(?d) {{
            "urn:eigenius:demo:institutions:delta_g": ?dg
        }}
        FIBER assay:validate_prediction {{
            candidate: "urn:eigenius:demo:institutions:dock_to_assay"(?d)
        }} AS ?v INTO "{target}"
        RETURN [] {{ d: ?d, v: ?v }}
        "#
    );

    let outcome =
        query::execute_with_into(&source, &data, runtime).expect("query executes with INTO");
    assert_eq!(
        outcome.into_resources.len(),
        1,
        "FIBER ... INTO should produce exactly one chain-bound resource"
    );
    let committed = &outcome.into_resources[0];
    assert_eq!(
        committed.id().map(|i| i.as_str()),
        Some(target),
        "committed resource carries the user-named INTO IRI"
    );
    // The response is a Verdict resource — verify the institution-
    // returned content survived stamping (ctor name + verdict_subject).
    let ctor = committed
        .get(&iri("urn:eigenius:core:ctor_name"))
        .and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        })
        .expect("ctor_name present on FIBER INTO Verdict");
    assert!(
        ctor == "Holds" || ctor == "Fails",
        "expected Holds or Fails ctor; got {ctor}"
    );
}

/// `WHERE ?v FAILS` filters the same setup the other way: ΔG=-8.5
/// produces a positive IC₅₀ (Holds), so the FAILS-projected row drops.
#[test]
fn eigenql_postfix_fails_drops_holding_row() {
    use eigenius_kernel::query;

    let (data, storage) = build_demo_data_layer();
    let index = build_demo_index(&data);
    let runtime = build_demo_runtime();
    let components = build_demo_components();
    let exec_ctx = build_exec_ctx(Arc::clone(&data), storage);

    let runtime = query::evaluate::FiberRuntime {
        index: Some(&index),
        runtime: Some(&runtime),
        components: Some(&components),
        overlay: None,
        ctx: Some(&exec_ctx),
        similarity: None,
        embedders: None,
        embedding_cache: None,
        vector_segment_cache: None,
    };

    let source = r#"
        USING INSTITUTION "urn:eigenius:demo:institutions:assay" AS assay
        USING NAMESPACE "urn:eigenius:demo:institutions:"

        MATCH "urn:eigenius:demo:institutions:DockingResult"(?d) {
            "urn:eigenius:demo:institutions:delta_g": ?dg
        }
        FIBER assay:validate_prediction {
            candidate: "urn:eigenius:demo:institutions:dock_to_assay"(?d)
        } AS ?v
        WHERE ?v FAILS
        RETURN [] {
            d: ?d
        }
    "#;

    let document = query::execute_with(source, &data, runtime).expect("query executes");
    let result_set = document
        .iter()
        .find(|r| {
            r.is_a()
                .iter()
                .any(|i| i.as_str() == "urn:eigenius:query:ResultSet")
        })
        .expect("ResultSet in document");
    let row_count = match result_set.get(&iri("urn:eigenius:query:row_count")) {
        Some(Value::Integer(n)) => *n,
        _ => panic!("ResultSet missing row_count"),
    };
    assert_eq!(row_count, 0, "FAILS should filter out the Holds row");
}

/// An unregistered comorphism IRI surfaces as a typed evaluation error
/// rather than silently identity-passing the source through.
#[test]
fn fiber_param_comorphism_coercion_unknown_comorphism_errors() {
    use eigenius_kernel::query::ast::{Expression, Name};
    use std::collections::BTreeMap;

    let (layer, storage) = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime();
    let components = build_demo_components();
    let exec_ctx = build_exec_ctx(Arc::clone(&layer), storage);

    let dock_iri = iri("urn:eigenius:demo:institutions:input1");
    let mut docking = Resource::new(dock_iri);
    docking.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(DOCKING_RESULT_CLASS.to_string())]),
    );
    docking.set(iri(DELTA_G_PROP), Value::Float(-8.5));
    let mut binding: BTreeMap<String, Value> = BTreeMap::new();
    binding.insert("d".into(), Value::Embedded(Box::new(docking)));

    let bogus_name = Name::FullIri(iri("urn:eigenius:demo:institutions:nonexistent_comorphism"));
    let source_expr =
        Expression::Variable(eigenius_kernel::query::ast::Variable { name: "d".into() });

    let err = eigenius_kernel::query::evaluate::eval_comorphism_coercion(
        &bogus_name,
        &source_expr,
        &binding,
        &layer,
        &index,
        &runtime,
        &components,
        &exec_ctx,
    )
    .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("not registered"),
        "expected `not registered` error; got: {msg}"
    );
}
