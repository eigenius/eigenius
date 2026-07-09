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

//! D14 §13.4 M8 — worked example end-to-end through WASM, auto-
//! registered from the layer chain.
//!
//! Mirror of `dock_assay_demo.rs` but:
//!
//! - The Dock / Assay `Institution` impls are loaded as `WasmInstitution`
//!   instances from the `examples/wasm-{dock,assay}` crates.
//! - The Arrhenius transformation is loaded as a `WasmComponent` from
//!   the `examples/wasm-arrhenius` crate.
//! - All three are auto-registered from the layer chain — the test
//!   builds a child layer carrying `runtime: wasm` + `wasm_binary`
//!   declarations and runs the same scan-and-register helpers the
//!   `EigeniusService` runs on commit. No manual `runtime.register()`
//!   or `components.register()` calls.
//!
//! Validates the "ontology-first" deployment promise: an institution
//! ships as a WASM binary embedded in an Eigon document, the kernel
//! installs it on Load, and dispatch routes through it transparently.
//! The dock/assay fixtures are built by `just build-wasm` and copied
//! into `kernel/tests/fixtures/`.

use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::capability::registration::{
    build_wasm_institution_runtime, scan_and_register,
};
use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::registry::InstitutionIndex;
use eigenius_kernel::institution::runtime::InstitutionRuntime;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::component::ComponentRegistry;

const DEMO_ONTOLOGY: &str = include_str!("../../ontologies/examples/dock-assay/dock-assay.json");

const DOCK_INST_IRI: &str = "urn:eigenius:demo:institutions:dock";
const ASSAY_INST_IRI: &str = "urn:eigenius:demo:institutions:assay";
const DOCKING_RESULT_CLASS: &str = "urn:eigenius:demo:institutions:DockingResult";
const ASSAY_PREDICTION_CLASS: &str = "urn:eigenius:demo:institutions:AssayPrediction";
const DELTA_G_PROP: &str = "urn:eigenius:demo:institutions:delta_g";
const IC50_PROP: &str = "urn:eigenius:demo:institutions:ic50";
const ARRHENIUS_COMPONENT_IRI: &str = "urn:eigenius:demo:institutions:cm_arrhenius";

const INSTITUTION_CLASS: &str = "urn:eigenius:institution:Institution";
const COMPONENT_CLASS: &str = "urn:eigenius:program:Component";
const INST_WASM_BINARY: &str = "urn:eigenius:institution:wasm_binary";
const COMP_IMPLEMENTATION: &str = "urn:eigenius:program:component:implementation";
const COMP_WASM_BINARY: &str = "urn:eigenius:program:component:wasm_binary";
const COMP_CAPABILITY_LEVEL: &str = "urn:eigenius:program:component:capability_level";
const CAPABILITY_PURE: &str = "urn:eigenius:program:capability_levels:pure";

const DOCK_FIXTURE: &[u8] = include_bytes!("fixtures/eigenius_wasm_dock.wasm");
const ASSAY_FIXTURE: &[u8] = include_bytes!("fixtures/eigenius_wasm_assay.wasm");
const ARRHENIUS_FIXTURE: &[u8] = include_bytes!("fixtures/eigenius_wasm_arrhenius.wasm");

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

/// Encode WASM bytes as a hex-prefixed string for embedding in an Eigon
/// resource via `urn:eigenius:institution:wasm_binary` or
/// `urn:eigenius:program:component:wasm_binary`. The registration scanner
/// recognises both base64 (default) and `hex:` prefix encodings.
fn embed_wasm(bytes: &[u8]) -> Value {
    Value::String(format!("hex:{}", hex::encode(bytes)))
}

/// Build a child layer that overrides the Dock / Assay institutions and
/// the cm_arrhenius component from the demo ontology with WASM-runtime
/// variants carrying inline `wasm_binary` declarations.
///
/// `all_resources()` is topmost-wins, so the child's `runtime: wasm`
/// declarations replace the parent's `runtime: in_process` ones. The
/// auto-registration scan in `build_wasm_institution_runtime` and
/// `scan_and_register` then picks up the WASM variants and constructs
/// the runtime + registry exactly as `EigeniusService` does on commit.
fn build_demo_layer() -> (Arc<Layer>, LayerStorage) {
    let ctx = bootstrap::bootstrap().expect("bootstrap kernel");
    let parent = Arc::clone(ctx.head());

    let mut base_builder = LayerBuilder::new("dock-assay-base", Some(parent));
    for r in eigon_json::parse_document(DEMO_ONTOLOGY).expect("parse demo ontology") {
        base_builder.add_resource(r).expect("add demo resource");
    }
    let base_layer = Arc::new(base_builder.build(LayerStorage::in_memory()));

    let mut wasm_builder = LayerBuilder::new("dock-assay-wasm", Some(base_layer));
    wasm_builder
        .add_resource(wasm_institution(DOCK_INST_IRI, "Dock", DOCK_FIXTURE))
        .expect("add Dock WASM override");
    wasm_builder
        .add_resource(wasm_institution(ASSAY_INST_IRI, "Assay", ASSAY_FIXTURE))
        .expect("add Assay WASM override");
    wasm_builder
        .add_resource(wasm_component(ARRHENIUS_COMPONENT_IRI, ARRHENIUS_FIXTURE))
        .expect("add Arrhenius WASM override");

    let storage = LayerStorage::in_memory();
    (Arc::new(wasm_builder.build(storage.clone())), storage)
}

fn wasm_institution(inst_iri: &str, name: &str, bytes: &[u8]) -> Resource {
    let mut r = Resource::new(iri(inst_iri));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(INSTITUTION_CLASS.to_string())]),
    );
    r.set(
        iri("urn:eigenius:institution:institution_iri"),
        Value::String(inst_iri.to_string()),
    );
    r.set(
        iri("urn:eigenius:institution:institution_name"),
        Value::String(name.to_string()),
    );
    r.set(
        iri(wk::RUNTIME),
        Value::String(wk::RUNTIME_WASM.to_string()),
    );
    r.set(iri(INST_WASM_BINARY), embed_wasm(bytes));
    r
}

fn wasm_component(component_iri: &str, bytes: &[u8]) -> Resource {
    let mut r = Resource::new(iri(component_iri));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(COMPONENT_CLASS.to_string())]),
    );
    r.set(iri(COMP_IMPLEMENTATION), Value::String("wasm".to_string()));
    r.set(
        iri(COMP_CAPABILITY_LEVEL),
        Value::String(CAPABILITY_PURE.to_string()),
    );
    r.set(iri(COMP_WASM_BINARY), embed_wasm(bytes));
    r
}

fn build_demo_index(layer: &Layer) -> Arc<InstitutionIndex> {
    let (idx, errors) = InstitutionIndex::from_layer(layer);
    assert!(errors.is_empty(), "demo ontology index errors: {errors:?}");
    Arc::new(idx)
}

/// Auto-register the WASM institutions declared in the layer chain.
fn build_demo_runtime(layer: &Layer) -> Arc<InstitutionRuntime> {
    let (runtime, report) = build_wasm_institution_runtime(layer);
    assert!(
        report.errors.is_empty(),
        "WASM institution registration errors: {:?}",
        report.errors
    );
    let registered: Vec<&str> = report
        .institutions_registered
        .iter()
        .map(|s| s.as_str())
        .collect();
    assert!(
        registered.contains(&DOCK_INST_IRI),
        "Dock not auto-registered: {registered:?}"
    );
    assert!(
        registered.contains(&ASSAY_INST_IRI),
        "Assay not auto-registered: {registered:?}"
    );
    Arc::new(runtime)
}

/// Auto-register the WASM components declared in the layer chain.
fn build_demo_components(layer: &Layer) -> Arc<ComponentRegistry> {
    let mut registry = ComponentRegistry::default();
    let result = scan_and_register(layer, &mut registry);
    assert!(
        result.report.errors.is_empty(),
        "WASM component registration errors: {:?}",
        result.report.errors
    );
    assert!(
        result
            .report
            .components_registered
            .iter()
            .any(|s| s == ARRHENIUS_COMPONENT_IRI),
        "cm_arrhenius not auto-registered: {:?}",
        result.report.components_registered
    );
    assert!(
        result.pending_io_components.is_empty(),
        "demo has no IO components, got {} pending",
        result.pending_io_components.len()
    );
    Arc::new(registry)
}

fn build_exec_ctx(layer: Arc<Layer>, storage: LayerStorage) -> ExecutionContext {
    ExecutionContext::new(
        layer,
        "dock-assay-wasm-demo",
        ExecutionMode::ReadOnly,
        storage,
    )
}

// ─── 1. Comorphism translation through the WASM host bridge ────────────

#[test]
fn wasm_comorphism_translates_dock_to_assay() {
    let (layer, _storage) = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime(&layer);
    let components = build_demo_components(&layer);

    let source = "
        namespace demo = \"urn:eigenius:demo:institutions\";

        program demo:translate : demo:DockingResult -> demo:AssayPrediction {
            demo:dock_to_assay(input)
        }
    ";

    let user_resources =
        eigenius_kernel::esl::compile_with_institutions(source, Arc::clone(&index))
            .expect("ESL compile");
    let mut user_builder =
        LayerBuilder::new("dock-assay-wasm-demo-program", Some(Arc::clone(&layer)));
    for r in user_resources {
        user_builder.add_resource(r).expect("add user resource");
    }
    let program_layer = Arc::new(user_builder.build(LayerStorage::in_memory()));

    let mut input = Resource::new(iri("urn:eigenius:demo:institutions:wasm_input1"));
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

    let (layer, _storage) = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime(&layer);
    let components = build_demo_components(&layer);
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

    let constraint = Constraint::Institution {
        iri: iri("urn:eigenius:demo:institutions:within_tolerance"),
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

    let (layer, storage) = build_demo_layer();
    let index = build_demo_index(&layer);
    let runtime = build_demo_runtime(&layer);
    let exec_ctx = build_exec_ctx(Arc::clone(&layer), storage);

    let mut good = Resource::new(iri("urn:eigenius:demo:institutions:wasm_good"));
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

    let mut bad = Resource::new(iri("urn:eigenius:demo:institutions:wasm_bad"));
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
