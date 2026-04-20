//! WasmComponent: a BuiltinComponent backed by a WASM Component Model binary.
//!
//! Hosts pure and read WASM components in the kernel via wasmtime's
//! Component Model API. The host:
//!   1. Loads a WASM component binary (built from WIT via cargo-component)
//!   2. Extracts the component's declared IRI by calling the `component-iri` export
//!   3. On each `execute` call, creates a fresh Store with fuel/memory limits,
//!      serializes input/argument to CBOR, calls the guest's `execute` export,
//!      and deserializes the result
//!
//! See D12 for the full specification.

use crate::layer::Layer;
use crate::ontology::eigon_cbor;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::program::component::{BuiltinComponent, ComponentResult};
use std::sync::Arc;
use wasmtime::component::types::ComponentFunc;
use wasmtime::component::{Component, Linker, Val};
use wasmtime::{Engine, Store};

/// Capability level determines which host imports are linked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilityLevel {
    /// Pure: no host imports beyond core type machinery.
    Pure,
    /// Read: adds `read-access` import (resolve resources from the layer chain).
    Read,
}

/// Host state carried by the Store. Provides access to the layer chain
/// for read-access host functions.
struct HostState {
    #[allow(dead_code)] // read when query-access and read-access imports are linked
    layer: Arc<Layer>,
}

/// Configuration for a WASM component instance.
#[derive(Debug, Clone)]
pub struct WasmComponentConfig {
    /// Maximum instructions executed per invocation (Wasmtime fuel).
    pub fuel_limit: u64,
    /// Maximum linear memory size in 64KB pages. Default 1024 = 64 MB.
    pub memory_limit_pages: u32,
}

impl Default for WasmComponentConfig {
    fn default() -> Self {
        Self {
            fuel_limit: 10_000_000,
            memory_limit_pages: 1024,
        }
    }
}

/// A BuiltinComponent backed by a WASM Component Model binary.
pub struct WasmComponent {
    engine: Engine,
    component: Component,
    component_iri: String,
    capability_level: CapabilityLevel,
    config: WasmComponentConfig,
}

impl WasmComponent {
    /// Load a WASM component from binary bytes.
    ///
    /// Compiles the component, then instantiates it once to read the
    /// declared IRI via the `component-iri` export. The component must
    /// conform to the `eigenius-component` world defined in
    /// `wit/eigenius-component.wit`.
    pub fn from_bytes(
        binary: &[u8],
        capability_level: CapabilityLevel,
        config: WasmComponentConfig,
    ) -> Result<Self, String> {
        let mut engine_config = wasmtime::Config::new();
        engine_config.wasm_component_model(true);
        engine_config.consume_fuel(true);

        let engine =
            Engine::new(&engine_config).map_err(|e| format!("engine creation failed: {e}"))?;

        let component = Component::from_binary(&engine, binary)
            .map_err(|e| format!("component compilation failed: {e}"))?;

        let component_iri = Self::extract_iri(&engine, &component, capability_level, &config)?;

        Ok(Self {
            engine,
            component,
            component_iri,
            capability_level,
            config,
        })
    }

    /// Get the component's declared IRI.
    pub fn iri(&self) -> &str {
        &self.component_iri
    }

    /// Get the component's capability level.
    pub fn capability_level(&self) -> CapabilityLevel {
        self.capability_level
    }

    /// Instantiate once to read the declared IRI via `component-iri`.
    fn extract_iri(
        engine: &Engine,
        component: &Component,
        capability_level: CapabilityLevel,
        config: &WasmComponentConfig,
    ) -> Result<String, String> {
        // Empty layer for the temporary instance — component-iri
        // shouldn't need resource resolution.
        let layer = Arc::new(crate::layer::LayerBuilder::new("empty", None).build());
        let linker = build_linker(engine, capability_level)?;

        let mut store = Store::new(engine, HostState { layer });
        store
            .set_fuel(config.fuel_limit)
            .map_err(|e| format!("set_fuel: {e}"))?;

        let instance = linker
            .instantiate(&mut store, component)
            .map_err(|e| format!("instantiation failed: {e}"))?;

        let iri_func = instance
            .get_func(&mut store, "component-iri")
            .ok_or_else(|| "component missing 'component-iri' export".to_string())?;

        let mut results = vec![Val::String(String::new())];
        iri_func
            .call(&mut store, &[], &mut results)
            .map_err(|e| format!("component-iri call failed: {e}"))?;

        match &results[0] {
            Val::String(s) => Ok(s.clone()),
            other => Err(format!("component-iri returned unexpected type: {other:?}")),
        }
    }
}

impl BuiltinComponent for WasmComponent {
    fn is_io(&self) -> bool {
        // Pure/read components hosted in the kernel are never IO.
        // IO components run in the orchestrator.
        false
    }

    fn execute(
        &self,
        input: &Resource,
        argument: Option<&Resource>,
        layer: &Layer,
    ) -> Result<ComponentResult, String> {
        let input_cbor = eigon_cbor::serialize_resource(input);
        let arg_cbor = match argument {
            Some(a) => eigon_cbor::serialize_resource(a),
            None => Vec::new(),
        };

        // Fresh instance per invocation (D12 §6.2).
        let mut store = Store::new(
            &self.engine,
            HostState {
                layer: Arc::new(layer.clone()),
            },
        );
        store
            .set_fuel(self.config.fuel_limit)
            .map_err(|e| format!("set_fuel: {e}"))?;

        let linker = build_linker(&self.engine, self.capability_level)?;
        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(|e| format!("instantiation failed: {e}"))?;

        let execute_func = instance
            .get_func(&mut store, "execute")
            .ok_or_else(|| "component missing 'execute' export".to_string())?;

        let input_val = Val::List(input_cbor.into_iter().map(Val::U8).collect());
        let arg_val = Val::List(arg_cbor.into_iter().map(Val::U8).collect());

        let mut results = vec![Val::Bool(false)]; // placeholder, will be overwritten
        execute_func
            .call(&mut store, &[input_val, arg_val], &mut results)
            .map_err(|e| format!("execute call failed: {e}"))?;

        parse_execute_result(&results[0])
    }
}

/// Build a Linker with host imports appropriate for the capability level.
fn build_linker(
    engine: &Engine,
    capability_level: CapabilityLevel,
) -> Result<Linker<HostState>, String> {
    let mut linker: Linker<HostState> = Linker::new(engine);

    if capability_level >= CapabilityLevel::Read {
        link_read_access(&mut linker)?;
    }

    Ok(linker)
}

/// Link the `eigenius:component/read-access@0.1.0` interface.
fn link_read_access(linker: &mut Linker<HostState>) -> Result<(), String> {
    let mut root = linker.root();
    let mut instance = root
        .instance("eigenius:component/read-access@0.1.0")
        .map_err(|e| format!("failed to create read-access instance: {e}"))?;

    // resolve: func(iri: string) -> option<list<u8>>
    instance
        .func_new("resolve", |ctx, _func: ComponentFunc, params, results| {
            let iri_str = match &params[0] {
                Val::String(s) => s.as_str(),
                other => {
                    return Err(wasmtime::Error::msg(format!(
                        "resolve: expected string param, got {other:?}"
                    )));
                }
            };

            let iri = match Iri::parse(iri_str) {
                Ok(i) => i,
                Err(_) => {
                    results[0] = Val::Option(None);
                    return Ok(());
                }
            };

            let layer = &ctx.data().layer;
            match layer.resolve(&iri) {
                Some(resource) => {
                    let cbor = eigon_cbor::serialize_resource(resource);
                    let bytes_val = Val::List(cbor.into_iter().map(Val::U8).collect());
                    results[0] = Val::Option(Some(Box::new(bytes_val)));
                }
                None => {
                    results[0] = Val::Option(None);
                }
            }

            Ok(())
        })
        .map_err(|e| format!("failed to link resolve: {e}"))?;

    Ok(())
}

/// Parse the `result<component-result, string>` returned by `execute`.
fn parse_execute_result(val: &Val) -> Result<ComponentResult, String> {
    let result = match val {
        Val::Result(r) => r,
        other => return Err(format!("expected result, got {other:?}")),
    };

    match result.as_ref() {
        Ok(Some(val)) => {
            // component-result is a record { output: list<u8> }
            let fields = match val.as_ref() {
                Val::Record(f) => f,
                other => return Err(format!("expected record, got {other:?}")),
            };

            let (name, output_val) = fields
                .first()
                .ok_or_else(|| "component-result has no fields".to_string())?;
            if name != "output" {
                return Err(format!("expected 'output' field, got '{name}'"));
            }

            let bytes = match output_val {
                Val::List(items) => items
                    .iter()
                    .map(|v| match v {
                        Val::U8(b) => Ok(*b),
                        other => Err(format!("expected u8, got {other:?}")),
                    })
                    .collect::<Result<Vec<u8>, String>>()?,
                other => return Err(format!("expected list<u8>, got {other:?}")),
            };

            let output = eigon_cbor::parse_resource_lenient(&bytes)
                .map_err(|e| format!("output CBOR parse failed: {e}"))?;

            Ok(ComponentResult {
                output,
                metrics: None,
            })
        }
        Ok(None) => Err("execute returned Ok(None)".to_string()),
        Err(Some(boxed)) => match boxed.as_ref() {
            Val::String(msg) => Err(msg.clone()),
            other => Err(format!("execute returned error: {other:?}")),
        },
        Err(None) => Err("execute returned Err(None)".to_string()),
    }
}
