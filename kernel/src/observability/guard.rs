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

//! RAII guard for RPC handler observability.
//!
//! `RpcGuard::start("kernel.rpc.foo")` emits a `debug` event for the
//! request and starts a timer. When the guard drops — at the end of
//! the handler, regardless of success / early-`?` / panic — it emits
//! a completion event carrying `latency_ms`. Optional `fail(kind)`
//! upgrades the completion event to `warn` and adds `error_kind`.
//!
//! This keeps every handler's instrumentation to one line at entry
//! plus (optional) one `guard.fail(...)` on the failure paths,
//! without having to repeat the log call at every return.

use crate::observability::field;

pub struct RpcGuard {
    operation: &'static str,
    started: std::time::Instant,
    error_kind: Option<&'static str>,
}

impl RpcGuard {
    /// Start a guard, emitting the `request received` debug event and
    /// recording the start time.
    pub fn start(operation: &'static str) -> Self {
        tracing::debug!({ field::OPERATION } = operation, "request received");
        Self {
            operation,
            started: std::time::Instant::now(),
            error_kind: None,
        }
    }

    /// Mark the request as failed with the given stable error kind.
    /// On drop, the guard will emit a `warn` completion event instead
    /// of the default `debug`. Last call wins if invoked more than
    /// once.
    pub fn fail(&mut self, kind: &'static str) {
        self.error_kind = Some(kind);
    }
}

impl Drop for RpcGuard {
    fn drop(&mut self) {
        let latency_ms = self.started.elapsed().as_millis() as u64;
        match self.error_kind {
            Some(kind) => tracing::warn!(
                { field::OPERATION } = self.operation,
                { field::LATENCY_MS } = latency_ms,
                { field::ERROR_KIND } = kind,
                "request failed"
            ),
            None => tracing::debug!(
                { field::OPERATION } = self.operation,
                { field::LATENCY_MS } = latency_ms,
                "request completed"
            ),
        }
    }
}
