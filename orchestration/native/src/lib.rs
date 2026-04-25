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

//! napi-rs native addon hosting IO WASM components for the Eigenius orchestrator.
//!
//! Exports:
//!   - `loadComponent(binary, opts)`   → compile + cache, returns a handle
//!   - `executeComponent(handle, input, argument, dispatch, resolve, query)` → run `execute`
//!   - `unloadComponent(handle)`       → release the compiled component
//!
//! The heavy lifting lives in the [`execute`] module behind a [`HostBridge`]
//! trait so tests can exercise the wasmtime path without a JS runtime.

#![deny(clippy::all)]

mod cache;
mod execute;
mod host_state;
mod linker;

#[cfg(test)]
mod tests;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunction;
use napi_derive::napi;
use std::sync::Arc;

use eigenius_wasm_runtime as wasm_rt;

use crate::host_state::{BridgeFuture, HostBridge};

// ---------------------------------------------------------------------------
// JS callback types (spread multi-arg via FnArgs, see spike REPORT.md)
// ---------------------------------------------------------------------------

type DispatchFn = ThreadsafeFunction<
    FnArgs<(String, Buffer, Buffer)>,
    Promise<Buffer>,
    FnArgs<(String, Buffer, Buffer)>,
    Status,
    false,
>;

type ResolveFn = ThreadsafeFunction<
    FnArgs<(String,)>,
    Promise<Option<Buffer>>,
    FnArgs<(String,)>,
    Status,
    false,
>;

type QueryFn = ThreadsafeFunction<
    FnArgs<(String,)>,
    Promise<Vec<Buffer>>,
    FnArgs<(String,)>,
    Status,
    false,
>;

struct NapiBridge {
    dispatch: DispatchFn,
    resolve: ResolveFn,
    query: QueryFn,
}

impl HostBridge for NapiBridge {
    fn dispatch<'a>(
        &'a self,
        iri: String,
        input: Vec<u8>,
        argument: Vec<u8>,
    ) -> BridgeFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let promise = self
                .dispatch
                .call_async(FnArgs::from((
                    iri,
                    Buffer::from(input),
                    Buffer::from(argument),
                )))
                .await
                .map_err(|e| format!("dispatch call_async: {e}"))?;
            let buf = promise.await.map_err(|e| format!("dispatch promise: {e}"))?;
            Ok(buf.to_vec())
        })
    }

    fn resolve<'a>(&'a self, iri: String) -> BridgeFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            let promise = self
                .resolve
                .call_async(FnArgs::from((iri,)))
                .await
                .map_err(|e| format!("resolve call_async: {e}"))?;
            let opt = promise.await.map_err(|e| format!("resolve promise: {e}"))?;
            Ok(opt.map(|b| b.to_vec()))
        })
    }

    fn query<'a>(&'a self, eigenql: String) -> BridgeFuture<'a, Vec<Vec<u8>>> {
        Box::pin(async move {
            let promise = self
                .query
                .call_async(FnArgs::from((eigenql,)))
                .await
                .map_err(|e| format!("query call_async: {e}"))?;
            let bufs = promise.await.map_err(|e| format!("query promise: {e}"))?;
            Ok(bufs.into_iter().map(|b| b.to_vec()).collect())
        })
    }
}

// ---------------------------------------------------------------------------
// JS-facing exports
// ---------------------------------------------------------------------------

#[napi(object)]
pub struct LoadOptions {
    /// Maximum wasmtime fuel per invocation. 0 → default.
    pub fuel_limit: f64,
    /// Memory cap in 64KB pages. 0 → default.
    pub memory_limit_pages: f64,
}

#[napi]
pub async fn load_component(wasm_binary: Buffer, opts: LoadOptions) -> Result<u32> {
    let binary = wasm_binary.to_vec();
    let config = resolve_config(&opts);
    tokio::task::spawn_blocking(move || execute::load(&binary, config))
        .await
        .map_err(|e| Error::from_reason(format!("spawn_blocking: {e}")))?
        .map_err(|e| Error::from_reason(format!("{e:#}")))
}

#[napi]
pub fn unload_component(handle: u32) -> bool {
    execute::unload(handle)
}

#[napi]
pub async fn execute_component(
    handle: u32,
    input: Buffer,
    argument: Buffer,
    dispatch: DispatchFn,
    resolve: ResolveFn,
    query: QueryFn,
) -> Result<Buffer> {
    let bridge: Arc<dyn HostBridge> = Arc::new(NapiBridge {
        dispatch,
        resolve,
        query,
    });
    let input_bytes = input.to_vec();
    let argument_bytes = argument.to_vec();
    let out = execute::execute(handle, input_bytes, argument_bytes, bridge)
        .await
        .map_err(|e| Error::from_reason(format!("{e:#}")))?;
    Ok(Buffer::from(out))
}

fn resolve_config(opts: &LoadOptions) -> wasm_rt::WasmComponentConfig {
    let defaults = wasm_rt::WasmComponentConfig::default();
    wasm_rt::WasmComponentConfig {
        fuel_limit: if opts.fuel_limit > 0.0 {
            opts.fuel_limit as u64
        } else {
            defaults.fuel_limit
        },
        memory_limit_pages: if opts.memory_limit_pages > 0.0 {
            opts.memory_limit_pages as u32
        } else {
            defaults.memory_limit_pages
        },
    }
}
