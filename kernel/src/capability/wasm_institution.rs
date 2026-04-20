//! WasmFiberReasoner: a FiberReasoner backed by a WASM Component Model binary.
//!
//! Hosts institution fiber reasoners (D10) as WASM components in the kernel.
//! Each invocation creates a fresh Wasmtime instance with fuel/memory limits
//! and read/query access to the layer chain.
//!
//! See D12 §4.4 for the WIT interface and §11 for the implementation plan.

use crate::context::ExecutionContext;
use crate::institution::error::{InstitutionError, MorphismValidation};
use crate::institution::{FiberDeclaration, FiberReasoner};
use crate::layer::Layer;
use crate::ontology::eigon_cbor;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};

use super::wasm_component::WasmComponentConfig;
use eigenius_wasm_runtime as wasm_rt;
use std::sync::Arc;
use wasmtime::component::types::ComponentFunc;
use wasmtime::component::{Component, Linker, Val};
use wasmtime::{Engine, Store};

/// Host state for a WASM institution. Institutions always get read + query
/// access but never IO.
struct HostState {
    #[allow(dead_code)] // read when read-access/query-access host funcs run
    layer: Arc<Layer>,
}

/// A FiberReasoner backed by a WASM Component Model binary.
pub struct WasmFiberReasoner {
    engine: Engine,
    component: Component,
    declaration: FiberDeclaration,
    config: WasmComponentConfig,
}

impl WasmFiberReasoner {
    /// Load a WASM institution from binary bytes.
    ///
    /// Compiles the component, then instantiates it once to read the
    /// FiberDeclaration. The component must conform to the
    /// `eigenius-institution` world defined in `wit/eigenius-component.wit`.
    pub fn from_bytes(binary: &[u8], config: WasmComponentConfig) -> Result<Self, String> {
        let engine = wasm_rt::new_engine().map_err(|e| format!("engine creation failed: {e}"))?;

        let component = wasm_rt::compile_component(&engine, binary)
            .map_err(|e| format!("component compilation failed: {e}"))?;

        let declaration = Self::extract_declaration(&engine, &component, &config)?;

        Ok(Self {
            engine,
            component,
            declaration,
            config,
        })
    }

    /// Get the institution IRI.
    pub fn institution_iri(&self) -> &Iri {
        &self.declaration.institution_iri
    }

    /// Instantiate once to read the fiber declaration.
    fn extract_declaration(
        engine: &Engine,
        component: &Component,
        config: &WasmComponentConfig,
    ) -> Result<FiberDeclaration, String> {
        let layer = Arc::new(crate::layer::LayerBuilder::new("empty", None).build());
        let linker = build_linker(engine)?;

        let mut store = Store::new(engine, HostState { layer });
        store
            .set_fuel(config.fuel_limit)
            .map_err(|e| format!("set_fuel: {e}"))?;

        let instance = linker
            .instantiate(&mut store, component)
            .map_err(|e| format!("instantiation failed: {e}"))?;

        let func = instance
            .get_func(&mut store, "fiber-declaration")
            .ok_or_else(|| "component missing 'fiber-declaration' export".to_string())?;

        let mut results = vec![Val::List(Vec::new())];
        func.call(&mut store, &[], &mut results)
            .map_err(|e| format!("fiber-declaration call failed: {e}"))?;

        let cbor = extract_bytes(&results[0])?;
        let decl_resource = eigon_cbor::parse_resource_lenient(&cbor)
            .map_err(|e| format!("fiber-declaration CBOR parse failed: {e}"))?;

        Self::parse_declaration(&decl_resource)
    }

    /// Parse a FiberDeclaration resource into the struct form.
    ///
    /// Expected properties on the resource:
    /// - `urn:eigenius:institution:institution_iri`: IRI string
    /// - `urn:eigenius:institution:institution_name`: human-readable name
    /// - `urn:eigenius:core:is_a` may include embedded morphism types, query types,
    ///   and structural properties (we simplify: accept them via dedicated properties).
    fn parse_declaration(resource: &Resource) -> Result<FiberDeclaration, String> {
        let iri_prop = Iri::parse("urn:eigenius:institution:institution_iri").unwrap();
        let name_prop = Iri::parse("urn:eigenius:institution:institution_name").unwrap();
        let morphism_types_prop = Iri::parse("urn:eigenius:institution:morphism_types").unwrap();
        let query_types_prop = Iri::parse("urn:eigenius:institution:query_types").unwrap();
        let structural_prop = Iri::parse("urn:eigenius:institution:structural_properties").unwrap();

        let institution_iri = match resource.get(&iri_prop) {
            Some(Value::String(s)) => {
                Iri::parse(s).map_err(|e| format!("invalid institution_iri: {e}"))?
            }
            _ => return Err("fiber-declaration missing institution_iri".to_string()),
        };

        let name = match resource.get(&name_prop) {
            Some(Value::String(s)) => s.clone(),
            _ => institution_iri.as_str().to_string(),
        };

        let morphism_types = collect_embedded(resource, &morphism_types_prop);
        let query_types = collect_embedded(resource, &query_types_prop);
        let structural_properties = collect_embedded(resource, &structural_prop);

        Ok(FiberDeclaration {
            institution_iri,
            name,
            morphism_types,
            query_types,
            structural_properties,
        })
    }

    /// Build a Store for a new invocation.
    fn fresh_store(&self, layer: Arc<Layer>) -> Result<Store<HostState>, String> {
        let mut store = Store::new(&self.engine, HostState { layer });
        store
            .set_fuel(self.config.fuel_limit)
            .map_err(|e| format!("set_fuel: {e}"))?;
        Ok(store)
    }

    /// Call a guest export that takes CBOR bytes and returns a `result<X, string>`.
    fn call_with_bytes(
        &self,
        export_name: &str,
        input_bytes: Vec<u8>,
        ctx: &ExecutionContext,
    ) -> Result<Vec<u8>, InstitutionError> {
        let layer = Arc::clone(ctx.head());
        let linker = build_linker(&self.engine).map_err(InstitutionError::ComputationFailed)?;
        let mut store = self
            .fresh_store(layer)
            .map_err(InstitutionError::ComputationFailed)?;

        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(|e| {
                InstitutionError::ComputationFailed(format!("instantiation failed: {e}"))
            })?;

        let func = instance.get_func(&mut store, export_name).ok_or_else(|| {
            InstitutionError::ComputationFailed(format!("component missing '{export_name}' export"))
        })?;

        let input_val = Val::List(input_bytes.into_iter().map(Val::U8).collect());
        let mut results = vec![Val::Bool(false)];
        func.call(&mut store, &[input_val], &mut results)
            .map_err(|e| {
                InstitutionError::ComputationFailed(format!("{export_name} call failed: {e}"))
            })?;

        parse_result_bytes(&results[0]).map_err(InstitutionError::ComputationFailed)
    }
}

impl FiberReasoner for WasmFiberReasoner {
    fn fiber_declaration(&self) -> FiberDeclaration {
        FiberDeclaration {
            institution_iri: self.declaration.institution_iri.clone(),
            name: self.declaration.name.clone(),
            morphism_types: self.declaration.morphism_types.clone(),
            query_types: self.declaration.query_types.clone(),
            structural_properties: self.declaration.structural_properties.clone(),
        }
    }

    fn query(
        &self,
        query: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        let bytes = eigon_cbor::serialize_resource(query);
        let out = self.call_with_bytes("query", bytes, ctx)?;
        eigon_cbor::parse_resource_lenient(&out).map_err(|e| {
            InstitutionError::ComputationFailed(format!("query output parse failed: {e}"))
        })
    }

    fn validate_morphism(
        &self,
        morphism: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<MorphismValidation, InstitutionError> {
        let layer = Arc::clone(ctx.head());
        let linker = build_linker(&self.engine).map_err(InstitutionError::ComputationFailed)?;
        let mut store = self
            .fresh_store(layer)
            .map_err(InstitutionError::ComputationFailed)?;

        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(|e| {
                InstitutionError::ComputationFailed(format!("instantiation failed: {e}"))
            })?;

        let func = instance
            .get_func(&mut store, "validate-morphism")
            .ok_or_else(|| {
                InstitutionError::ComputationFailed(
                    "component missing 'validate-morphism' export".into(),
                )
            })?;

        let bytes = eigon_cbor::serialize_resource(morphism);
        let input_val = Val::List(bytes.into_iter().map(Val::U8).collect());
        let mut results = vec![Val::Bool(false)];
        func.call(&mut store, &[input_val], &mut results)
            .map_err(|e| {
                InstitutionError::ComputationFailed(format!("validate-morphism call failed: {e}"))
            })?;

        parse_validation_result(&results[0]).map_err(InstitutionError::ComputationFailed)
    }

    fn discover_morphisms(
        &self,
        resources: &[Resource],
        ctx: &ExecutionContext,
    ) -> Result<Vec<Resource>, InstitutionError> {
        let layer = Arc::clone(ctx.head());
        let linker = build_linker(&self.engine).map_err(InstitutionError::ComputationFailed)?;
        let mut store = self
            .fresh_store(layer)
            .map_err(InstitutionError::ComputationFailed)?;

        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(|e| {
                InstitutionError::ComputationFailed(format!("instantiation failed: {e}"))
            })?;

        let func = instance
            .get_func(&mut store, "discover-morphisms")
            .ok_or_else(|| {
                InstitutionError::ComputationFailed(
                    "component missing 'discover-morphisms' export".into(),
                )
            })?;

        let input_list: Vec<Val> = resources
            .iter()
            .map(|r| {
                let bytes = eigon_cbor::serialize_resource(r);
                Val::List(bytes.into_iter().map(Val::U8).collect())
            })
            .collect();
        let input_val = Val::List(input_list);

        let mut results = vec![Val::Bool(false)];
        func.call(&mut store, &[input_val], &mut results)
            .map_err(|e| {
                InstitutionError::ComputationFailed(format!("discover-morphisms call failed: {e}"))
            })?;

        let bytes_list =
            parse_result_list(&results[0]).map_err(InstitutionError::ComputationFailed)?;
        let mut out = Vec::with_capacity(bytes_list.len());
        for bytes in bytes_list {
            out.push(eigon_cbor::parse_resource_lenient(&bytes).map_err(|e| {
                InstitutionError::ComputationFailed(format!("morphism CBOR parse failed: {e}"))
            })?);
        }
        Ok(out)
    }
}

/// Build a Linker with read-access and query-access imports (institutions
/// never get io-access).
fn build_linker(engine: &Engine) -> Result<Linker<HostState>, String> {
    let mut linker: Linker<HostState> = Linker::new(engine);
    link_read_access(&mut linker)?;
    link_query_access(&mut linker)?;
    Ok(linker)
}

fn link_read_access(linker: &mut Linker<HostState>) -> Result<(), String> {
    let mut root = linker.root();
    let mut instance = root
        .instance("eigenius:component/read-access@0.1.0")
        .map_err(|e| format!("failed to create read-access instance: {e}"))?;

    instance
        .func_new("resolve", |ctx, _f: ComponentFunc, params, results| {
            let iri_str = match &params[0] {
                Val::String(s) => s.as_str(),
                other => {
                    return Err(wasmtime::Error::msg(format!(
                        "resolve: expected string, got {other:?}"
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

            match ctx.data().layer.resolve(&iri) {
                Some(resource) => {
                    let cbor = eigon_cbor::serialize_resource(resource);
                    let bytes_val = Val::List(cbor.into_iter().map(Val::U8).collect());
                    results[0] = Val::Option(Some(Box::new(bytes_val)));
                }
                None => results[0] = Val::Option(None),
            }
            Ok(())
        })
        .map_err(|e| format!("failed to link resolve: {e}"))?;

    Ok(())
}

fn link_query_access(linker: &mut Linker<HostState>) -> Result<(), String> {
    let mut root = linker.root();
    let mut instance = root
        .instance("eigenius:component/query-access@0.1.0")
        .map_err(|e| format!("failed to create query-access instance: {e}"))?;

    instance
        .func_new("query", |_ctx, _f: ComponentFunc, _params, results| {
            // Phase 8 stub — return empty result list.
            // Full EigenQL dispatch across the WASM boundary is future work
            // (requires streaming results and query evaluator integration).
            results[0] = Val::Result(Ok(Some(Box::new(Val::List(Vec::new())))));
            Ok(())
        })
        .map_err(|e| format!("failed to link query: {e}"))?;

    Ok(())
}

/// Extract list<u8> from a Val.
fn extract_bytes(val: &Val) -> Result<Vec<u8>, String> {
    match val {
        Val::List(items) => items
            .iter()
            .map(|v| match v {
                Val::U8(b) => Ok(*b),
                other => Err(format!("expected u8, got {other:?}")),
            })
            .collect(),
        other => Err(format!("expected list<u8>, got {other:?}")),
    }
}

/// Parse a `result<list<u8>, string>` return value into bytes.
fn parse_result_bytes(val: &Val) -> Result<Vec<u8>, String> {
    let r = match val {
        Val::Result(r) => r,
        other => return Err(format!("expected result, got {other:?}")),
    };

    match r.as_ref() {
        Ok(Some(v)) => extract_bytes(v.as_ref()),
        Ok(None) => Err("Ok(None) in result".to_string()),
        Err(Some(boxed)) => match boxed.as_ref() {
            Val::String(msg) => Err(msg.clone()),
            other => Err(format!("error value: {other:?}")),
        },
        Err(None) => Err("Err(None) in result".to_string()),
    }
}

/// Parse `result<list<list<u8>>, string>` into `Vec<Vec<u8>>`.
fn parse_result_list(val: &Val) -> Result<Vec<Vec<u8>>, String> {
    let r = match val {
        Val::Result(r) => r,
        other => return Err(format!("expected result, got {other:?}")),
    };

    match r.as_ref() {
        Ok(Some(v)) => match v.as_ref() {
            Val::List(items) => items.iter().map(extract_bytes).collect(),
            other => Err(format!("expected list, got {other:?}")),
        },
        Ok(None) => Ok(Vec::new()),
        Err(Some(boxed)) => match boxed.as_ref() {
            Val::String(msg) => Err(msg.clone()),
            other => Err(format!("error value: {other:?}")),
        },
        Err(None) => Err("Err(None) in result".to_string()),
    }
}

/// Parse the `result<tuple<validation-result, string>, string>` return
/// from validate-morphism.
fn parse_validation_result(val: &Val) -> Result<MorphismValidation, String> {
    let r = match val {
        Val::Result(r) => r,
        other => return Err(format!("expected result, got {other:?}")),
    };

    match r.as_ref() {
        Ok(Some(v)) => match v.as_ref() {
            Val::Tuple(items) => {
                let variant = match &items[0] {
                    Val::Enum(name) => name.as_str(),
                    other => return Err(format!("expected enum, got {other:?}")),
                };
                let reason = match &items[1] {
                    Val::String(s) => s.clone(),
                    _ => String::new(),
                };

                match variant {
                    "valid" => Ok(MorphismValidation::Valid),
                    "invalid" => Ok(MorphismValidation::Invalid(reason)),
                    "undecidable" => Ok(MorphismValidation::Undecidable),
                    other => Err(format!("unknown validation-result variant: {other}")),
                }
            }
            other => Err(format!("expected tuple, got {other:?}")),
        },
        Ok(None) => Err("Ok(None) in validate-morphism result".to_string()),
        Err(Some(boxed)) => match boxed.as_ref() {
            Val::String(msg) => Err(msg.clone()),
            other => Err(format!("error value: {other:?}")),
        },
        Err(None) => Err("Err(None) in result".to_string()),
    }
}

/// Collect embedded resources from an array-valued property.
fn collect_embedded(resource: &Resource, prop: &Iri) -> Vec<Resource> {
    match resource.get(prop) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::Embedded(r) => Some(r.as_ref().clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}
