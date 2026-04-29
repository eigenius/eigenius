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

//! D14 §13.4 M8 — worked example end-to-end through WASM.
//!
//! Mirror of `d14_dock_assay_demo.rs` with the in-process Rust
//! `Institution` impls replaced by `WasmInstitution` instances loaded
//! from the `examples/wasm-d14-{dock,assay,arrhenius}` crates and the
//! Arrhenius `BuiltinComponent` replaced by a `WasmComponent`. Same
//! ontology, same four scenarios, same Verdicts; the difference is
//! that every dispatch crosses the WASM host bridge.
//!
//! Validates `WasmInstitution` (D14 §11) + `WasmComponent` (D12)
//! routing under a real domain. The dock/assay fixtures are built by
//! `just build-wasm` and copied into `kernel/tests/fixtures/`.

use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::capability::wasm_component::{
    CapabilityLevel, WasmComponent, WasmComponentConfig,
};
use eigenius_kernel::capability::wasm_institution_d14::WasmInstitution;
use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::registry::InstitutionIndex;
use eigenius_kernel::institution::runtime::InstitutionRuntime;
use eigenius_kernel::layer::{Layer, LayerBuilder};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::component::ComponentRegistry;

const DEMO_ONTOLOGY: &str =
    include_str!("../../ontologies/examples/d14-dock-assay/dock-assay.json");

const DOCK_INST_IRI: &str = "urn:eigenius:demo:d14:dock";
const ASSAY_INST_IRI: &str = "urn:eigenius:demo:d14:assay";
const DOCKING_RESULT_CLASS: &str = "urn:eigenius:demo:d14:DockingResult";
const ASSAY_PREDICTION_CLASS: &str = "urn:eigenius:demo:d14:AssayPrediction";
const DELTA_G_PROP: &str = "urn:eigenius:demo:d14:delta_g";
const IC50_PROP: &str = "urn:eigenius:demo:d14:ic50";
const ARRHENIUS_COMPONENT_IRI: &str = "urn:eigenius:demo:d14:cm_arrhenius";

const DOCK_FIXTURE: &[u8] = include_bytes!("fixtures/eigenius_wasm_d14_dock.wasm");
const ASSAY_FIXTURE: &[u8] = include_bytes!("fixtures/eigenius_wasm_d14_assay.wasm");
const ARRHENIUS_FIXTURE: &[u8] = include_bytes!("fixtures/eigenius_wasm_d14_arrhenius.wasm");

const RT_KCAL_PER_MOL: f64 = 0.616;
const IC50_SCALE_NM: f64 = 1.0e9;

fn arrhenius_ic50_nm(delta_g_kcal: f64) -> f64 {
    (-delta_g_kcal / RT_KCAL_PER_MOL).exp() * IC50_SCALE_NM
}

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

fn build_demo_layer() -> Arc<Layer> {
    let ctx = bootstrap::bootstrap().expect("bootstrap kernel");
    let parent = Arc::clone(ctx.head());
    let mut builder = LayerBuilder::new("d14-dock-assay-wasm-demo", Some(parent));
    for r in eigon_json::parse_document(DEMO_ONTOLOGY).expect("parse demo ontology") {
        builder.add_resource(r).expect("add demo resource");
    }
    Arc::new(builder.build())
}

fn build_demo_index(layer: &Layer) -> Arc<InstitutionIndex> {
    let (idx, errors) = InstitutionIndex::from_layer(layer);
    assert!(errors.is_empty(), "demo ontology index errors: {errors:?}");
    Arc::new(idx)
}

/// Build the InstitutionRuntime by loading the dock + assay WASM
/// fixtures via `WasmInstitution::from_bytes`.
fn build_demo_runtime() -> Arc<InstitutionRuntime> {
    let mut runtime = InstitutionRuntime::new();
    runtime
        .register(Box::new(
            WasmInstitution::from_bytes(
                iri(DOCK_INST_IRI),
                DOCK_FIXTURE,
                WasmComponentConfig::default(),
            )
            .expect("load dock WASM fixture"),
        ))
        .expect("register Dock WasmInstitution");
    runtime
        .register(Box::new(
            WasmInstitution::from_bytes(
                iri(ASSAY_INST_IRI),
                ASSAY_FIXTURE,
                WasmComponentConfig::default(),
            )
            .expect("load assay WASM fixture"),
        ))
        .expect("register Assay WasmInstitution");
    Arc::new(runtime)
}

/// Build the ComponentRegistry by loading the Arrhenius WASM fixture
/// as a Pure-capability `WasmComponent`.
fn build_demo_components() -> Arc<ComponentRegistry> {
    let mut registry = ComponentRegistry::default();
    let component = WasmComponent::from_bytes(
        ARRHENIUS_FIXTURE,
        CapabilityLevel::Pure,
        WasmComponentConfig::default(),
    )
    .expect("load arrhenius WASM fixture");
    registry.register(ARRHENIUS_COMPONENT_IRI.to_string(), Box::new(component));
    Arc::new(registry)
}

fn build_exec_ctx(layer: Arc<Layer>) -> ExecutionContext {
    ExecutionContext::new(layer, "d14-wasm-demo", ExecutionMode::ReadOnly)
}

// ─── 1. Comorphism translation through the WASM host bridge ────────────

#[test]
fn wasm_comorphism_translates_dock_to_assay() {
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
    let mut user_builder = LayerBuilder::new("d14-wasm-demo-program", Some(Arc::clone(&layer)));
    for r in user_resources {
        user_builder.add_resource(r).expect("add user resource");
    }
    let program_layer = Arc::new(user_builder.build());

    let mut input = Resource::new(iri("urn:eigenius:demo:d14:wasm_input1"));
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
    .expect("WASM comorphism dispatch");

    let ic50 = as_float(result.output.get(&iri(IC50_PROP))).expect("AssayPrediction.ic50");
    let expected = arrhenius_ic50_nm(-8.5);
    assert!(
        (ic50 - expected).abs() < expected * 1e-6,
        "WASM IC50≈{expected}, got {ic50}"
    );
    assert!(
        result
            .output
            .is_a()
            .iter()
            .any(|i| i.as_str() == ASSAY_PREDICTION_CLASS),
        "translated resource should be an AssayPrediction; got is_a={:?}",
        result.output.is_a()
    );
}

// ─── 2. Decidable QueryClass dispatch through WASM ─────────────────────

fn run_within_tolerance(
    predicted: f64,
    target: f64,
    tolerance: f64,
) -> eigenius_kernel::nbe::val::Val {
    use eigenius_kernel::nbe::env::Rho;
    use eigenius_kernel::nbe::eval::{eval_ctx, EvalCtx};
    use eigenius_kernel::nbe::term::{Constraint, Exp};
    use std::sync::Mutex;

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

    let constraint = Constraint::Institution {
        iri: iri("urn:eigenius:demo:d14:within_tolerance"),
        args: vec![
            wrap_float(predicted),
            wrap_float(target),
            wrap_float(tolerance),
        ],
    };
    let exp = Exp::NativeDecide(constraint, Box::new(Exp::Unit));
    eval_ctx(&exp, &Rho::Nil, &ctx).expect("WASM decide eval")
}

#[test]
fn wasm_decidable_holds_in_tolerance() {
    use eigenius_kernel::nbe::val::Val;
    let v = run_within_tolerance(500.0, 600.0, 200.0);
    assert!(matches!(v, Val::Refl(_)), "expected Refl(Unit), got {v:?}");
}

#[test]
fn wasm_decidable_fails_outside_tolerance() {
    use eigenius_kernel::nbe::val::{Neut, Val};
    let v = run_within_tolerance(500.0, 600.0, 50.0);
    match v {
        Val::Nt(Neut::Gen(_, name)) => assert_eq!(name, "__constraint_failed"),
        other => panic!("expected failing neutral, got {other:?}"),
    }
}

// ─── 3. AutoOnLoad dispatch through WASM ───────────────────────────────

#[test]
fn wasm_auto_on_load_fires_on_assay_prediction() {
    use eigenius_kernel::institution::dispatch::dispatch_auto_on_load_for_resource;

    let layer = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime();
    let exec_ctx = build_exec_ctx(Arc::clone(&layer));

    let mut good = Resource::new(iri("urn:eigenius:demo:d14:wasm_good"));
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

    let mut bad = Resource::new(iri("urn:eigenius:demo:d14:wasm_bad"));
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
