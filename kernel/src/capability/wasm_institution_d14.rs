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

//! Host bridge between the kernel's [`Institution`] trait (D14 §8) and
//! a WASM Component Model binary targeting the
//! `eigenius-institution-d14` world.
//!
//! [`WasmInstitution`] compiles the component once at construction
//! time and instantiates a fresh Wasmtime store per dispatch. Each
//! call goes:
//!
//! ```text
//!   Institution::extract_typed → WIT export `extract-typed` →
//!     CBOR-encoded resource-data → guest extract → typed-value bytes →
//!     parsed Val (currently always Val::ResourceVal)
//! ```
//!
//! and analogously for `reify` and `query`.
//!
//! The Mini-TT typed-value codec is intentionally minimal at this
//! milestone (M4): a typed-value at the boundary is a CBOR-encoded
//! Eigon resource, parsed back as `Val::ResourceVal`. M5 extends this
//! to cover Mini-TT primitives, tuples, and inductive values once the
//! `Exp::InstitutionInvoke` evaluator actually exercises them. Until
//! then, an institution's payload types are expressible as resources
//! carrying the value as a property — sufficient for the smoke-test
//! example and for sketching out comorphism dispatch end-to-end.

use crate::context::ExecutionContext;
use crate::institution::error::InstitutionError;
use crate::institution::runtime::Institution;
use crate::layer::Layer;
use crate::nbe::val::Val;
use crate::ontology::eigon_cbor;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;

use super::wasm_component::WasmComponentConfig;
use eigenius_wasm_runtime as wasm_rt;
use std::sync::Arc;
use wasmtime::component::types::ComponentFunc;
use wasmtime::component::{Component, Linker, Val as WasmVal};
use wasmtime::{Engine, Store};

/// Host state for a WASM institution targeting the D14 world.
struct HostState {
    #[allow(dead_code)] // read by the read-access / query-access host funcs
    layer: Arc<Layer>,
}

/// A D14-shape institution backed by a WASM Component Model binary.
///
/// Constructed with the institution IRI at install time — the binary
/// itself does *not* declare an institution IRI (declarations are
/// ontology-first under D14). The kernel finds the binary by IRI in
/// the runtime registry and dispatches via the trait methods.
pub struct WasmInstitution {
    institution_iri: Iri,
    engine: Engine,
    component: Component,
    config: WasmComponentConfig,
}

impl WasmInstitution {
    /// Compile a WASM Component Model binary as a D14 institution
    /// keyed by `institution_iri`.
    pub fn from_bytes(
        institution_iri: Iri,
        binary: &[u8],
        config: WasmComponentConfig,
    ) -> Result<Self, String> {
        let engine = wasm_rt::new_engine().map_err(|e| format!("engine creation failed: {e}"))?;
        let component = wasm_rt::compile_component(&engine, binary)
            .map_err(|e| format!("component compilation failed: {e}"))?;
        Ok(Self {
            institution_iri,
            engine,
            component,
            config,
        })
    }

    fn fresh_store(&self, layer: Arc<Layer>) -> Result<Store<HostState>, String> {
        let mut store = Store::new(&self.engine, HostState { layer });
        store
            .set_fuel(self.config.fuel_limit)
            .map_err(|e| format!("set_fuel: {e}"))?;
        Ok(store)
    }

    /// Call a guest export of shape
    /// `(iri, list<u8>) -> result<list<u8>, string>`. The two boundary
    /// methods (`extract-typed`, `reify`) and the reasoning method
    /// (`query`) all match this shape under the D14 WIT world.
    fn call_iri_bytes(
        &self,
        export_name: &str,
        procedure_iri: &Iri,
        payload: Vec<u8>,
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
            InstitutionError::ComputationFailed(format!("component missing `{export_name}` export"))
        })?;

        let payload_val = WasmVal::List(payload.into_iter().map(WasmVal::U8).collect());
        let params = [
            WasmVal::String(procedure_iri.as_str().to_string()),
            payload_val,
        ];
        let mut results = vec![WasmVal::Bool(false)];
        func.call(&mut store, &params, &mut results).map_err(|e| {
            InstitutionError::ComputationFailed(format!("{export_name} call failed: {e}"))
        })?;

        parse_result_bytes(&results[0]).map_err(InstitutionError::ComputationFailed)
    }
}

impl Institution for WasmInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.institution_iri
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        let payload = eigon_cbor::serialize_resource(resource);
        let out = self.call_iri_bytes("extract-typed", procedure_iri, payload, ctx)?;
        // M4: typed-value is encoded as a CBOR resource. M5 extends to
        // primitives / inductives when the evaluator exercises them.
        let parsed = eigon_cbor::parse_resource_lenient(&out).map_err(|e| {
            InstitutionError::ComputationFailed(format!("extract-typed output parse failed: {e}"))
        })?;
        Ok(Val::ResourceVal(Box::new(parsed)))
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        value: &Val,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        // M4 marshalling restriction: only Val::ResourceVal is
        // exchanged across the WASM boundary. Any other Val variant
        // surfaces as a typed error so M5 sees a clear failure when
        // the evaluator first hands us a primitive / inductive.
        let resource = match value {
            Val::ResourceVal(r) => r.as_ref().clone(),
            other => {
                return Err(InstitutionError::ComputationFailed(format!(
                    "reify: M4 marshalling supports only Val::ResourceVal payloads; got {other:?}"
                )));
            }
        };
        let payload = eigon_cbor::serialize_resource(&resource);
        let out = self.call_iri_bytes("reify", procedure_iri, payload, ctx)?;
        eigon_cbor::parse_resource_lenient(&out).map_err(|e| {
            InstitutionError::ComputationFailed(format!("reify output parse failed: {e}"))
        })
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        let payload = eigon_cbor::serialize_resource(input);
        let out = self.call_iri_bytes("query", procedure_iri, payload, ctx)?;
        eigon_cbor::parse_resource_lenient(&out).map_err(|e| {
            InstitutionError::ComputationFailed(format!("query output parse failed: {e}"))
        })
    }
}

// ─── Wasmtime linker / boundary helpers ────────────────────────────────

/// Build a Linker with `read-access` and `query-access` imports —
/// institutions get layer-chain visibility and EigenQL dispatch but
/// never IO. Mirrors the helper in [`super::wasm_institution`]; M8
/// dedupes once the legacy bridge retires.
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
                WasmVal::String(s) => s.as_str(),
                other => {
                    return Err(wasmtime::Error::msg(format!(
                        "resolve: expected string, got {other:?}"
                    )));
                }
            };
            let iri = match Iri::parse(iri_str) {
                Ok(i) => i,
                Err(_) => {
                    results[0] = WasmVal::Option(None);
                    return Ok(());
                }
            };
            match ctx.data().layer.resolve(&iri) {
                Some(resource) => {
                    let cbor = eigon_cbor::serialize_resource(resource);
                    let bytes_val = WasmVal::List(cbor.into_iter().map(WasmVal::U8).collect());
                    results[0] = WasmVal::Option(Some(Box::new(bytes_val)));
                }
                None => results[0] = WasmVal::Option(None),
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
            // Same M4 stub as the legacy bridge — full EigenQL dispatch
            // across the WASM boundary is post-D14 work.
            results[0] = WasmVal::Result(Ok(Some(Box::new(WasmVal::List(Vec::new())))));
            Ok(())
        })
        .map_err(|e| format!("failed to link query: {e}"))?;
    Ok(())
}

fn extract_bytes(val: &WasmVal) -> Result<Vec<u8>, String> {
    match val {
        WasmVal::List(items) => items
            .iter()
            .map(|v| match v {
                WasmVal::U8(b) => Ok(*b),
                other => Err(format!("expected u8, got {other:?}")),
            })
            .collect(),
        other => Err(format!("expected list<u8>, got {other:?}")),
    }
}

fn parse_result_bytes(val: &WasmVal) -> Result<Vec<u8>, String> {
    let r = match val {
        WasmVal::Result(r) => r,
        other => return Err(format!("expected result, got {other:?}")),
    };
    match r.as_ref() {
        Ok(Some(v)) => extract_bytes(v.as_ref()),
        Ok(None) => Err("Ok(None) in result".to_string()),
        Err(Some(boxed)) => match boxed.as_ref() {
            WasmVal::String(msg) => Err(msg.clone()),
            other => Err(format!("error value: {other:?}")),
        },
        Err(None) => Err("Err(None) in result".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExecutionMode;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::Value;

    const FIXTURE: &[u8] =
        include_bytes!("../../../kernel/tests/fixtures/eigenius_wasm_d14_echo.wasm");

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn make_ctx() -> ExecutionContext {
        let layer = Arc::new(LayerBuilder::new("empty", None).build());
        ExecutionContext::new(layer, "test", ExecutionMode::ReadOnly)
    }

    fn load_echo() -> WasmInstitution {
        WasmInstitution::from_bytes(
            iri("urn:eigenius:test:d14_echo"),
            FIXTURE,
            WasmComponentConfig::default(),
        )
        .expect("load d14 echo fixture")
    }

    #[test]
    fn institution_iri_round_trips() {
        let inst = load_echo();
        assert_eq!(
            inst.institution_iri().as_str(),
            "urn:eigenius:test:d14_echo"
        );
    }

    #[test]
    fn extract_typed_dispatches_with_procedure_iri() {
        let inst = load_echo();
        let ctx = make_ctx();
        let mut input = Resource::new(iri("urn:eigenius:test:input"));
        input.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("hello".into()),
        );

        let val = inst
            .extract_typed(&iri("urn:eigenius:test:proc:p1"), &input, &ctx)
            .expect("extract_typed");
        let resource = match val {
            Val::ResourceVal(r) => *r,
            other => panic!("expected ResourceVal, got {other:?}"),
        };
        // Echo institution stamps `provenance` with the procedure IRI
        // and `stage` with the export it was dispatched on.
        let provenance = resource
            .get(&iri("urn:eigenius:test:d14_echo:provenance"))
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(provenance.as_deref(), Some("urn:eigenius:test:proc:p1"));
        let stage = resource
            .get(&iri("urn:eigenius:test:d14_echo:stage"))
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(stage.as_deref(), Some("extract_typed"));
    }

    #[test]
    fn reify_round_trips_resource_val_payload() {
        let inst = load_echo();
        let ctx = make_ctx();
        let mut payload = Resource::new(iri("urn:eigenius:test:payload"));
        payload.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("payload".into()),
        );

        let result = inst
            .reify(
                &iri("urn:eigenius:test:proc:p2"),
                &Val::ResourceVal(Box::new(payload)),
                &ctx,
            )
            .expect("reify");
        let stage = result
            .get(&iri("urn:eigenius:test:d14_echo:stage"))
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(stage.as_deref(), Some("reify"));
    }

    #[test]
    fn reify_rejects_non_resource_val_payload() {
        let inst = load_echo();
        let ctx = make_ctx();
        let err = inst
            .reify(&iri("urn:eigenius:test:proc:p3"), &Val::Unit, &ctx)
            .expect_err("Val::Unit unsupported by M4 marshalling");
        match err {
            InstitutionError::ComputationFailed(msg) => {
                assert!(msg.contains("M4 marshalling"), "unexpected reason: {msg}");
            }
            other => panic!("expected ComputationFailed, got {other:?}"),
        }
    }

    #[test]
    fn query_dispatches_with_procedure_iri() {
        let inst = load_echo();
        let ctx = make_ctx();
        let mut input = Resource::new(iri("urn:eigenius:test:q_input"));
        input.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("question".into()),
        );

        let result = inst
            .query(&iri("urn:eigenius:test:proc:check_q"), &input, &ctx)
            .expect("query");
        let stage = result
            .get(&iri("urn:eigenius:test:d14_echo:stage"))
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(stage.as_deref(), Some("query"));
        let provenance = result
            .get(&iri("urn:eigenius:test:d14_echo:provenance"))
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(
            provenance.as_deref(),
            Some("urn:eigenius:test:proc:check_q")
        );
    }

    #[test]
    fn missing_export_surfaces_typed_error() {
        // Construct a separate WasmInstitution from the *legacy*
        // ordering-institution fixture (which targets the old WIT
        // world and so does not export `extract-typed`) and verify
        // the host bridge surfaces a clear ComputationFailed rather
        // than panicking.
        let legacy_bytes = include_bytes!(
            "../../../kernel/tests/fixtures/eigenius_wasm_ordering_institution.wasm"
        );
        let inst = WasmInstitution::from_bytes(
            iri("urn:eigenius:test:legacy_targeted"),
            legacy_bytes,
            WasmComponentConfig::default(),
        )
        .expect("legacy fixture compiles even though it doesn't export D14 surface");

        let ctx = make_ctx();
        let r = Resource::new(iri("urn:eigenius:test:any"));
        let err = inst
            .extract_typed(&iri("urn:eigenius:test:proc:any"), &r, &ctx)
            .expect_err("legacy fixture should not export `extract-typed`");
        match err {
            InstitutionError::ComputationFailed(msg) => {
                assert!(
                    msg.contains("extract-typed"),
                    "expected ComputationFailed mentioning the export name; got {msg}"
                );
            }
            other => panic!("expected ComputationFailed, got {other:?}"),
        }
    }
}
