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

//! `eigenius-schemaorg-worker` — the schema.org converter as an Eigenius runtime
//! worker (D60 §4.1, D57 Level 2). Speaks the substrate's CBOR RPC over a Unix
//! domain socket, identical in shape to `eigenius-test-worker` and the R worker:
//! spawn → `Health` → `DispatchMethod` → `Evict`. `DispatchMethod` runs the real
//! `eigenius_schemaorg::convert` over the pinned input and returns the
//! conversion-report `Resource` as Eigon-CBOR (the dispatch body lives in the lib
//! so it is unit-testable in-process).
//!
//! Configuration via env (set by the substrate at spawn):
//! - `EIGENIUS_WORKER_UDS` (required) — path the worker binds.
//! - `EIGENIUS_RUNTIME_ENV_DIGEST` / `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH` /
//!   `EIGENIUS_RUNTIME_ENV_DIR` (D26 §9.3 cross-check) — same contract as the
//!   other workers.

use eigenius_runtime_substrate::cross_check::{
    self, CrossCheckError, EXIT_CODE_CROSS_CHECK_FAILURE,
};
use eigenius_runtime_substrate::rpc::client::{server_recv_request, server_send_response};
use eigenius_runtime_substrate::rpc::codec::MAX_FRAME_SIZE_DEFAULT;
use eigenius_runtime_substrate::rpc::protocol::{HealthInfo, NumericalMetadata, Request, Response};
use eigenius_schemaorg_worker::run_conversion;
use serde_bytes::ByteBuf;
use std::env;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::ExitCode;

const UDS_ENV: &str = "EIGENIUS_WORKER_UDS";

fn main() -> ExitCode {
    // D26 §9.3 bootstrap cross-check: refuse to bind the UDS if the env vs
    // in-image manifest hash disagree (exit 78 → the substrate surfaces
    // WorkerCrossCheckFailed).
    if let Err(e) = cross_check::verify_in_worker() {
        report_cross_check_failure(&e);
        return ExitCode::from(EXIT_CODE_CROSS_CHECK_FAILURE as u8);
    }

    let uds_path = match env::var(UDS_ENV) {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            eprintln!("eigenius-schemaorg-worker: {UDS_ENV} not set");
            return ExitCode::from(2);
        }
    };

    let _ = std::fs::remove_file(&uds_path);
    let listener = match UnixListener::bind(&uds_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "eigenius-schemaorg-worker: bind {} failed: {e}",
                uds_path.display()
            );
            return ExitCode::from(3);
        }
    };
    {
        // World-rw so the substrate (a different UID than the container) can
        // connect(); the per-invocation tempdir owns access control.
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&uds_path, std::fs::Permissions::from_mode(0o666))
        {
            eprintln!(
                "eigenius-schemaorg-worker: chmod 0o666 on {} failed: {e}",
                uds_path.display()
            );
            return ExitCode::from(3);
        }
    }

    loop {
        let mut stream = match listener.accept() {
            Ok((s, _addr)) => s,
            Err(e) => {
                eprintln!("eigenius-schemaorg-worker: accept failed: {e}");
                return ExitCode::from(4);
            }
        };
        match serve(&mut stream) {
            ServeOutcome::EvictReceived => return ExitCode::SUCCESS,
            ServeOutcome::ConnectionClosed => continue,
            ServeOutcome::FatalError(code) => return ExitCode::from(code),
        }
    }
}

enum ServeOutcome {
    EvictReceived,
    ConnectionClosed,
    FatalError(u8),
}

fn serve(stream: &mut UnixStream) -> ServeOutcome {
    loop {
        let req = match server_recv_request(stream, MAX_FRAME_SIZE_DEFAULT) {
            Ok(Some(r)) => r,
            Ok(None) => return ServeOutcome::ConnectionClosed,
            Err(e) => {
                eprintln!("eigenius-schemaorg-worker: recv failed: {e}");
                return ServeOutcome::FatalError(5);
            }
        };
        let exit_after = matches!(req, Request::Evict);
        let resp = handle(req);
        if let Err(e) = server_send_response(stream, &resp) {
            eprintln!("eigenius-schemaorg-worker: send failed: {e}");
            return ServeOutcome::FatalError(6);
        }
        if exit_after {
            return ServeOutcome::EvictReceived;
        }
    }
}

fn report_cross_check_failure(err: &CrossCheckError) {
    eprintln!("eigenius-schemaorg-worker: bootstrap cross-check failed: {err}");
}

fn handle(req: Request) -> Response {
    match req {
        Request::Health => Response::Health(HealthInfo {
            manifest_hash_in_image: env::var("EIGENIUS_RUNTIME_ENV_MANIFEST_HASH").ok(),
            env_digest_in_image: env::var("EIGENIUS_RUNTIME_ENV_DIGEST").ok(),
            numerical_metadata: NumericalMetadata {
                host_kernel: Some("schemaorg".to_string()),
                ..Default::default()
            },
        }),
        Request::Instantiate { .. } => Response::Instantiated { ready: true },
        Request::RegisterMirror { mirror_iri, .. } => Response::MirrorRegistered { mirror_iri },
        Request::DispatchMethod {
            invocation_id,
            target_kind: _,
            target: _,
            inputs,
        } => dispatch_convert(invocation_id, inputs),
        Request::Evict => Response::Evicted,
    }
}

fn dispatch_convert(invocation_id: String, inputs: Vec<ByteBuf>) -> Response {
    match run_conversion(&inputs) {
        Ok(output_cbor) => Response::DispatchOk {
            invocation_id,
            output: ByteBuf::from(output_cbor),
            derivations: Vec::new(),
            dispatched_to: None,
        },
        Err(message) => Response::DispatchFailed {
            invocation_id,
            error_kind: "runtime_error".to_string(),
            message,
        },
    }
}
