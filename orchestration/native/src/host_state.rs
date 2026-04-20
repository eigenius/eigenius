//! Per-invocation wasmtime Store data + the host-callback trait.
//!
//! `HostBridge` abstracts the three async host imports (dispatch-component,
//! resolve, query). The napi-rs layer implements it via `ThreadsafeFunction`;
//! tests implement it with plain closures.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type BridgeResult<T> = Result<T, String>;
pub type BridgeFuture<'a, T> = Pin<Box<dyn Future<Output = BridgeResult<T>> + Send + 'a>>;

pub trait HostBridge: Send + Sync {
    fn dispatch<'a>(
        &'a self,
        iri: String,
        input: Vec<u8>,
        argument: Vec<u8>,
    ) -> BridgeFuture<'a, Vec<u8>>;

    fn resolve<'a>(&'a self, iri: String) -> BridgeFuture<'a, Option<Vec<u8>>>;

    fn query<'a>(&'a self, eigenql: String) -> BridgeFuture<'a, Vec<Vec<u8>>>;
}

pub struct HostState {
    pub bridge: Arc<dyn HostBridge>,
}
