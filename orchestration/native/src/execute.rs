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

//! Core load + execute flow, independent of napi-rs.
//!
//! Tests can call these functions directly with a synthetic `HostBridge`;
//! the napi layer wraps them and supplies a `ThreadsafeFunction`-backed
//! bridge.

use anyhow::Result;
use dashmap::DashMap;
use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use wasmtime::component::Component;
use wasmtime::{Engine, Store};

use eigenius_wasm_runtime as wasm_rt;

use crate::cache;
use crate::host_state::{HostBridge, HostState};
use crate::linker;

pub type Handle = u32;

pub struct LoadedComponent {
    pub component: Component,
    pub config: wasm_rt::WasmComponentConfig,
}

// ---------------------------------------------------------------------------
// Process-global state
// ---------------------------------------------------------------------------

pub fn engine() -> Result<&'static Engine> {
    static E: OnceCell<Engine> = OnceCell::new();
    E.get_or_try_init(|| wasm_rt::new_engine().map_err(|e| anyhow::anyhow!("engine init: {e}")))
}

fn registry() -> &'static DashMap<Handle, Arc<LoadedComponent>> {
    static R: OnceCell<DashMap<Handle, Arc<LoadedComponent>>> = OnceCell::new();
    R.get_or_init(DashMap::new)
}

fn next_handle() -> Handle {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed) as Handle
}

// ---------------------------------------------------------------------------
// Public API used by both the napi layer and tests
// ---------------------------------------------------------------------------

pub fn load(binary: &[u8], config: wasm_rt::WasmComponentConfig) -> Result<Handle> {
    let engine = engine()?;
    let hash = cache::hash_binary(binary);

    let component = match cache::try_load(engine, &hash) {
        Ok(Some(c)) => c,
        Ok(None) => compile_and_cache(engine, binary, &hash)?,
        Err(e) => {
            eprintln!("eigenius-orchestrator-wasm: cache load failed, recompiling: {e:#}");
            let _ = cache::evict(&hash);
            // Recompile and re-populate the cache so the bad entry self-heals.
            compile_and_cache(engine, binary, &hash)?
        }
    };

    let handle = next_handle();
    registry().insert(handle, Arc::new(LoadedComponent { component, config }));
    Ok(handle)
}

fn compile_and_cache(engine: &Engine, binary: &[u8], hash: &str) -> Result<Component> {
    let c = wasm_rt::compile_component(engine, binary)
        .map_err(|e| anyhow::anyhow!("compile: {e}"))?;
    if let Err(e) = cache::store(&c, hash) {
        eprintln!("eigenius-orchestrator-wasm: cache store failed: {e:#}");
    }
    Ok(c)
}

pub fn unload(handle: Handle) -> bool {
    registry().remove(&handle).is_some()
}

pub async fn execute(
    handle: Handle,
    input: Vec<u8>,
    argument: Vec<u8>,
    bridge: Arc<dyn HostBridge>,
) -> Result<Vec<u8>> {
    let loaded = registry()
        .get(&handle)
        .map(|e| Arc::clone(e.value()))
        .ok_or_else(|| anyhow::anyhow!("unknown component handle: {handle}"))?;

    let engine = engine()?;
    let linker = linker::build_io_linker(engine)?;

    let mut store = Store::new(engine, HostState { bridge });
    store.set_fuel(loaded.config.fuel_limit)?;

    let instance = linker
        .instantiate_async(&mut store, &loaded.component)
        .await?;

    let func = instance
        .get_func(&mut store, "execute")
        .ok_or_else(|| anyhow::anyhow!("component missing 'execute' export"))?;

    let params = wasm_rt::encode_execute_params(&input, &argument);
    let mut results = vec![wasmtime::component::Val::Bool(false)];
    func.call_async(&mut store, &params, &mut results).await?;

    wasm_rt::parse_execute_result(&results[0]).map_err(|e| anyhow::anyhow!(e))
}
