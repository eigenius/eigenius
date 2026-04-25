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

//! Shared WASM runtime primitives for the Eigenius kernel and orchestrator.
//!
//! This crate contains the pieces of wasmtime + Component Model plumbing that
//! are identical on both sides of the kernel/orchestrator split. Linker setup
//! and host state stay with each caller — their host imports and async-ness
//! differ too much to share.
//!
//! See `docs/design/d12b-orchestrator-wasm-plan.md` §3 for the surface rationale.

use wasmtime::component::{Component, Val};
use wasmtime::Engine;

/// Capability level determines which host imports a component may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilityLevel {
    /// No host imports beyond core type machinery.
    Pure,
    /// Adds `read-access` (resolve resources from the layer chain).
    Read,
    /// Adds `query-access` and `io-access`. Orchestrator-hosted only.
    Io,
}

/// Configuration for a WASM component instance.
#[derive(Debug, Clone)]
pub struct WasmComponentConfig {
    /// Maximum instructions executed per invocation (wasmtime fuel).
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

/// Build a wasmtime `Config` with the settings Eigenius requires.
///
/// Note: `async_support` became a default-on no-op in wasmtime 43, so we no
/// longer toggle it — both the kernel (sync) and orchestrator (async) paths
/// use the same config.
pub fn engine_config() -> wasmtime::Config {
    let mut cfg = wasmtime::Config::new();
    cfg.wasm_component_model(true);
    cfg.consume_fuel(true);
    cfg
}

/// Shortcut: build a `Config` and wrap it in an `Engine`.
pub fn new_engine() -> wasmtime::Result<Engine> {
    Engine::new(&engine_config())
}

/// Compile a Component Model binary.
pub fn compile_component(engine: &Engine, binary: &[u8]) -> wasmtime::Result<Component> {
    Component::from_binary(engine, binary)
}

/// Encode the `(input, argument)` arguments expected by the guest's `execute`
/// export: `(list<u8>, list<u8>)`.
pub fn encode_execute_params(input: &[u8], argument: &[u8]) -> Vec<Val> {
    vec![
        Val::List(input.iter().copied().map(Val::U8).collect()),
        Val::List(argument.iter().copied().map(Val::U8).collect()),
    ]
}

/// Parse the `result<component-result, string>` returned by `execute`.
///
/// `component-result` is `record { output: list<u8> }`. Returns the raw
/// `output` bytes on success. Caller is responsible for CBOR-decoding them
/// into whatever resource representation they use.
pub fn parse_execute_result(val: &Val) -> Result<Vec<u8>, String> {
    let result = match val {
        Val::Result(r) => r,
        other => return Err(format!("expected result, got {other:?}")),
    };

    match result.as_ref() {
        Ok(Some(val)) => {
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

            match output_val {
                Val::List(items) => items
                    .iter()
                    .map(|v| match v {
                        Val::U8(b) => Ok(*b),
                        other => Err(format!("expected u8, got {other:?}")),
                    })
                    .collect::<Result<Vec<u8>, String>>(),
                other => Err(format!("expected list<u8>, got {other:?}")),
            }
        }
        Ok(None) => Err("execute returned Ok(None)".to_string()),
        Err(Some(boxed)) => match boxed.as_ref() {
            Val::String(msg) => Err(msg.clone()),
            other => Err(format!("execute returned error: {other:?}")),
        },
        Err(None) => Err("execute returned Err(None)".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_config_builds() {
        let _ = engine_config();
    }

    #[test]
    fn encode_execute_params_shape() {
        let vals = encode_execute_params(&[1, 2, 3], &[9]);
        assert_eq!(vals.len(), 2);
        match (&vals[0], &vals[1]) {
            (Val::List(a), Val::List(b)) => {
                assert_eq!(a.len(), 3);
                assert_eq!(b.len(), 1);
            }
            other => panic!("unexpected shape: {other:?}"),
        }
    }

    #[test]
    fn default_config_is_reasonable() {
        let c = WasmComponentConfig::default();
        assert!(c.fuel_limit > 0);
        assert!(c.memory_limit_pages > 0);
    }
}
