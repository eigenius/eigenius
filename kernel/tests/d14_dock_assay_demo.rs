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
//! Plumbing-only end-to-end test of the D14 institution surface, using
//! the dock→assay scenario from D14 §5.1. One source institution
//! (Dock), one target institution (Assay), one comorphism
//! (`dock_to_assay`) with a real transformation Component middle
//! (`cm_arrhenius` — the Arrhenius approximation IC₅₀ ≈ exp(-ΔG/RT)),
//! plus two QueryClasses against the assay institution: a Decidable
//! `within_tolerance` predicate and an AutoOnLoad
//! `assay_prediction_validity` check fired on AssayPrediction Load.
//!
//! Each `#[test]` exercises one D14 dispatch path:
//!
//! - [`comorphism_translates_dock_to_assay`] — `Exp::InstitutionInvoke`,
//!   four-step pipeline (D14 §9.3).
//! - [`decidable_query_class_holds_in_tolerance`] /
//!   [`decidable_query_class_fails_outside_tolerance`] —
//!   `Exp::NativeDecide` against a Decidable QueryClass (D14 §9.2).
//! - [`auto_on_load_fires_on_assay_prediction`] — Load-time dispatch
//!   for an AutoOnLoad QueryClass (D14 §9.1).
//!
//! WASM packaging of the institutions and transformation is a follow-on;
//! this test wires them as in-process Rust impls so the demo is
//! self-contained and hermetic.

use std::sync::Arc;
use std::sync::Mutex;

use eigenius_kernel::bootstrap;
use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::registry::InstitutionIndex;
use eigenius_kernel::institution::runtime::{Institution, InstitutionRuntime};
use eigenius_kernel::layer::{Layer, LayerBuilder};
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::component::{BuiltinComponent, ComponentRegistry, ComponentResult};

// ─── Constants from the demo ontology ──────────────────────────────────

const DEMO_ONTOLOGY: &str =
    include_str!("../../ontologies/examples/d14-dock-assay/dock-assay.json");

const DOCK_INST_IRI: &str = "urn:eigenius:demo:d14:dock";
const ASSAY_INST_IRI: &str = "urn:eigenius:demo:d14:assay";
const DOCKING_RESULT_CLASS: &str = "urn:eigenius:demo:d14:DockingResult";
const ASSAY_PREDICTION_CLASS: &str = "urn:eigenius:demo:d14:AssayPrediction";
const DELTA_G_PROP: &str = "urn:eigenius:demo:d14:delta_g";
const IC50_PROP: &str = "urn:eigenius:demo:d14:ic50";
const PREDICTED_IC50_PROP: &str = "urn:eigenius:demo:d14:predicted_ic50";
const TARGET_IC50_PROP: &str = "urn:eigenius:demo:d14:target_ic50";
const TOLERANCE_PROP: &str = "urn:eigenius:demo:d14:tolerance";
const EXTRACT_DG_PROC: &str = "urn:eigenius:demo:d14:proc:extract_dg";
const REIFY_IC50_PROC: &str = "urn:eigenius:demo:d14:proc:reify_ic50";
const WITHIN_TOLERANCE_PROC: &str = "urn:eigenius:demo:d14:proc:within_tolerance";
const CHECK_ASSAY_PREDICTION_PROC: &str = "urn:eigenius:demo:d14:proc:check_assay_prediction";
const ARRHENIUS_COMPONENT_IRI: &str = "urn:eigenius:demo:d14:cm_arrhenius";

const DEMO_LAYER_NAME: &str = "d14-dock-assay-demo";

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

/// Build the demo layer on top of the bootstrap chain.
fn build_demo_layer() -> Arc<Layer> {
    let ctx = bootstrap::bootstrap().expect("bootstrap kernel");
    let parent = Arc::clone(ctx.head());
    let mut builder = LayerBuilder::new(DEMO_LAYER_NAME, Some(parent));
    let resources = eigon_json::parse_document(DEMO_ONTOLOGY).expect("parse demo ontology");
    for r in resources {
        builder.add_resource(r).expect("add demo resource");
    }
    Arc::new(builder.build())
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

fn build_exec_ctx(layer: Arc<Layer>) -> ExecutionContext {
    ExecutionContext::new(layer, "d14-demo", ExecutionMode::ReadOnly)
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
    ) -> Result<Resource, InstitutionError> {
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
        // synthesises an input resource carrying a positional
        // `decide_args` array. Unpack predicted/target/tolerance from
        // that array. As a fall-back, also accept the named-property
        // shape (used when the same handler is reachable via FIBER
        // with explicit parameters).
        let decide_args_iri = iri("urn:eigenius:institution:decide_args");
        let from_args = match input.get(&decide_args_iri) {
            Some(Value::Array(items)) if items.len() == 3 => {
                let arg_float = |idx: usize| match &items[idx] {
                    Value::Float(f) => Some(*f),
                    Value::Integer(n) => Some(*n as f64),
                    Value::Embedded(r) => first_float_property(r),
                    _ => None,
                };
                Some((arg_float(0), arg_float(1), arg_float(2)))
            }
            _ => None,
        };
        let (predicted, target, tolerance) = from_args.unwrap_or_else(|| {
            (
                as_float(input.get(&iri(PREDICTED_IC50_PROP))),
                as_float(input.get(&iri(TARGET_IC50_PROP))),
                as_float(input.get(&iri(TOLERANCE_PROP))),
            )
        });
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
    ) -> Result<Resource, InstitutionError> {
        match procedure_iri.as_str() {
            WITHIN_TOLERANCE_PROC => {
                let ctor = Self::within_tolerance_verdict(input);
                Ok(Self::verdict_resource(ctor))
            }
            CHECK_ASSAY_PREDICTION_PROC => {
                let ctor = Self::assay_prediction_verdict(input);
                Ok(Self::verdict_resource(ctor))
            }
            _ => Err(InstitutionError::UnknownType(format!(
                "assay institution does not implement procedure `{procedure_iri}`"
            ))),
        }
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
    let layer = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime();
    let components = build_demo_components();

    let source = "
        namespace demo = \"urn:eigenius:demo:d14\";

        program demo:translate : demo:DockingResult -> demo:AssayPrediction {
            demo:dock_to_assay(input)
        }
    ";

    let user_resources =
        eigenius_kernel::esl::compile_with_institutions(source, Arc::clone(&index))
            .expect("ESL compile");
    let mut user_builder = LayerBuilder::new("d14-demo-program", Some(Arc::clone(&layer)));
    for r in user_resources {
        user_builder.add_resource(r).expect("add user resource");
    }
    let program_layer = Arc::new(user_builder.build());

    // Build a sample DockingResult: ΔG = -8.5 kcal/mol.
    let mut input = Resource::new(iri("urn:eigenius:demo:d14:input1"));
    input.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(DOCKING_RESULT_CLASS.to_string())]),
    );
    input.set(iri(DELTA_G_PROP), Value::Float(-8.5));

    let prog_iri = iri("urn:eigenius:demo:d14:translate");
    let program = program_layer
        .resolve(&prog_iri)
        .expect("translate program in layer")
        .clone();

    let result = eigenius_kernel::program::eval_io::execute_program_nbe_with_institutions_d14(
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
}

// ─── 2. Decidable QueryClass dispatch (D14 §9.2) ───────────────────────

/// Build a `Constraint::Institution` that calls `within_tolerance` with
/// three Float arguments. Returns the program's eval result (Refl on
/// Holds, neutral on Fails / Undecidable).
fn run_within_tolerance(predicted: f64, target: f64, tolerance: f64) -> Val {
    use eigenius_kernel::nbe::env::Rho;
    use eigenius_kernel::nbe::eval::{eval_ctx, EvalCtx};
    use eigenius_kernel::nbe::term::{Constraint, Exp, PrimitiveType};

    let layer = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime();
    let components = build_demo_components();
    let dispatched_traces = Arc::new(Mutex::new(Vec::new()));

    let ctx = EvalCtx::IO {
        layer: Arc::clone(&layer),
        registry: components,
        trace_store: None,
        dispatched_traces,
        task_context: None,
        institution_index: Some(index),
        institution_runtime: Some(runtime),
    };

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
        iri: iri("urn:eigenius:demo:d14:within_tolerance"),
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

    let layer = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime();
    let exec_ctx = build_exec_ctx(Arc::clone(&layer));

    // Healthy AssayPrediction — IC₅₀ = 250 nM.
    let mut good = Resource::new(iri("urn:eigenius:demo:d14:good_prediction"));
    good.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(ASSAY_PREDICTION_CLASS.to_string())]),
    );
    good.set(iri(IC50_PROP), Value::Float(250.0));
    let errs = dispatch_auto_on_load_for_resource(&good, &index, &runtime, &exec_ctx);
    assert!(
        errs.is_empty(),
        "Holds should produce no AutoOnLoad errors; got {errs:?}"
    );

    // Broken AssayPrediction — non-positive IC₅₀ should Fail the
    // AutoOnLoad check, surfacing as a typed ValidationError.
    let mut bad = Resource::new(iri("urn:eigenius:demo:d14:bad_prediction"));
    bad.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(ASSAY_PREDICTION_CLASS.to_string())]),
    );
    bad.set(iri(IC50_PROP), Value::Float(-1.0));
    let errs = dispatch_auto_on_load_for_resource(&bad, &index, &runtime, &exec_ctx);
    assert_eq!(errs.len(), 1, "expected one Fails error; got {errs:?}");
    assert!(
        errs[0].message.contains("returned Fails"),
        "unexpected message: {}",
        errs[0].message
    );
}
