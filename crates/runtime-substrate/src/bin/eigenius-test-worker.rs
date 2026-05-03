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

//! `eigenius-test-worker` — bash-backed smoke-test worker.
//!
//! Speaks the substrate's CBOR RPC ([`eigenius_runtime_substrate::rpc`])
//! over a Unix domain socket. Used as the integration fixture for the
//! Phase 18a substrate skeleton — exercises the full
//! spawn → connect → dispatch → exit path without dragging in a real
//! interpreter.
//!
//! Protocol convention for `dispatch_method`:
//!
//! - `target` — CBOR-encoded `String`: a bash one-liner
//! - `inputs` — ignored in v1
//! - `output` — CBOR-encoded `String`: stdout of the bash invocation
//!
//! Production-grade per-language workers (Phase 19+) marshal Eigon
//! resources at this boundary; the test worker uses raw strings so the
//! smoke test stays decoupled from resource serialization.
//!
//! Configuration via env vars:
//!
//! - `EIGENIUS_TEST_WORKER_UDS` (required) — path the worker binds to
//! - `EIGENIUS_RUNTIME_ENV_DIGEST` (optional) — echoed back on `health`
//! - `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH` (optional) — same

use eigenius_runtime_substrate::rpc::client::{server_recv_request, server_send_response};
use eigenius_runtime_substrate::rpc::codec::MAX_FRAME_SIZE_DEFAULT;
use eigenius_runtime_substrate::rpc::protocol::{HealthInfo, NumericalMetadata, Request, Response};
use serde_bytes::ByteBuf;
use std::env;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let uds_path = match env::var("EIGENIUS_TEST_WORKER_UDS") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            eprintln!("eigenius-test-worker: EIGENIUS_TEST_WORKER_UDS not set");
            return ExitCode::from(2);
        }
    };

    // Stale socket file from a previous worker run blocks `bind`. The
    // worker owns this path so removing is safe.
    let _ = std::fs::remove_file(&uds_path);
    let listener = match UnixListener::bind(&uds_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "eigenius-test-worker: bind {} failed: {e}",
                uds_path.display()
            );
            return ExitCode::from(3);
        }
    };

    let mut stream = match listener.accept() {
        Ok((s, _addr)) => s,
        Err(e) => {
            eprintln!("eigenius-test-worker: accept failed: {e}");
            return ExitCode::from(4);
        }
    };

    serve(&mut stream)
}

fn serve(stream: &mut UnixStream) -> ExitCode {
    loop {
        let req = match server_recv_request(stream, MAX_FRAME_SIZE_DEFAULT) {
            Ok(Some(r)) => r,
            // Clean EOF — the substrate dropped the connection.
            Ok(None) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("eigenius-test-worker: recv failed: {e}");
                return ExitCode::from(5);
            }
        };

        let exit_after = matches!(req, Request::Evict);
        let resp = handle(req);
        if let Err(e) = server_send_response(stream, &resp) {
            eprintln!("eigenius-test-worker: send failed: {e}");
            return ExitCode::from(6);
        }
        if exit_after {
            return ExitCode::SUCCESS;
        }
    }
}

fn handle(req: Request) -> Response {
    match req {
        Request::Health => Response::Health(HealthInfo {
            manifest_hash_in_image: env::var("EIGENIUS_RUNTIME_ENV_MANIFEST_HASH").ok(),
            env_digest_in_image: env::var("EIGENIUS_RUNTIME_ENV_DIGEST").ok(),
            numerical_metadata: NumericalMetadata {
                host_kernel: Some("test-runtime".to_string()),
                ..Default::default()
            },
        }),
        Request::Instantiate { .. } => Response::Instantiated { ready: true },
        Request::RegisterMirror { mirror_iri, .. } => Response::MirrorRegistered { mirror_iri },
        Request::DispatchMethod {
            invocation_id,
            target,
            inputs: _,
        } => dispatch_bash(invocation_id, target),
        Request::Evict => Response::Evicted,
    }
}

fn dispatch_bash(invocation_id: String, target: ByteBuf) -> Response {
    let script: String = match ciborium::from_reader(&target[..]) {
        Ok(s) => s,
        Err(e) => {
            return Response::DispatchFailed {
                invocation_id,
                error_kind: "method_signature_mismatch".to_string(),
                message: format!("expected target to be CBOR-encoded String: {e}"),
            };
        }
    };

    let output = match Command::new("bash").arg("-c").arg(&script).output() {
        Ok(o) if o.status.success() => o.stdout,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
            return Response::DispatchFailed {
                invocation_id,
                error_kind: "runtime_error".to_string(),
                message: format!(
                    "bash exited with status {:?}: {}",
                    o.status.code(),
                    stderr.trim()
                ),
            };
        }
        Err(e) => {
            return Response::DispatchFailed {
                invocation_id,
                error_kind: "runtime_error".to_string(),
                message: format!("could not spawn bash: {e}"),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output).into_owned();
    let mut output_cbor = Vec::new();
    if let Err(e) = ciborium::into_writer(&stdout, &mut output_cbor) {
        return Response::DispatchFailed {
            invocation_id,
            error_kind: "runtime_error".to_string(),
            message: format!("could not encode output as CBOR: {e}"),
        };
    }

    Response::DispatchOk {
        invocation_id,
        output: ByteBuf::from(output_cbor),
        dispatched_to: None,
    }
}
