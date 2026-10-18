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

//! Substrate-side RPC client wrapping a `UnixStream`.
//!
//! Phase 18a ships a minimal *synchronous* client. The substrate calls
//! into it from the orchestrator's napi addon (where napi-rs's threaded
//! pool absorbs blocking I/O) and from per-language tests. Async
//! variants can wrap this when production workloads warrant it; for the
//! v1 spawn-per-invocation model the blocking shape is sufficient.

use crate::rpc::codec::{decode_frame, encode_frame, FrameError, MAX_FRAME_SIZE_DEFAULT};
use crate::rpc::protocol::{Request, Response};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;
use thiserror::Error;

/// Failure modes for [`WorkerRpcClient`] operations.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Wire framing or CBOR codec failed.
    #[error("frame error: {0}")]
    Frame(#[from] FrameError),

    /// The peer closed the connection cleanly between frames where a
    /// response was expected. Distinct from a transport error.
    #[error("worker closed the connection before responding")]
    PeerClosed,
}

/// Substrate-side RPC client for one worker.
///
/// Owns the `UnixStream` and a CBOR codec that frames each message
/// with a 4-byte length prefix (see [`crate::rpc::codec`]). One
/// in-flight request at a time — the protocol is request/response,
/// no streaming or interleaving in v1.
pub struct WorkerRpcClient {
    stream: UnixStream,
    max_frame_size: usize,
}

impl WorkerRpcClient {
    /// Wrap an already-opened stream. Typically the stream comes from
    /// [`crate::spawner::WorkerSpawner::attach_uds`] after spawning a
    /// worker.
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            max_frame_size: MAX_FRAME_SIZE_DEFAULT,
        }
    }

    /// Override the per-frame ceiling. Defaults to
    /// [`MAX_FRAME_SIZE_DEFAULT`]. Useful for tests or for deployments
    /// that need to ship larger mirror archives.
    pub fn with_max_frame_size(mut self, max: usize) -> Self {
        self.max_frame_size = max;
        self
    }

    /// Apply read and write timeouts to the underlying stream. Both
    /// directions get the same timeout. Pass `None` to clear.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.stream.set_read_timeout(timeout)?;
        self.stream.set_write_timeout(timeout)?;
        Ok(())
    }

    /// Send a request and wait for the response. Returns
    /// [`ClientError::PeerClosed`] if the worker closed cleanly
    /// without replying — typically an internal worker error before
    /// it could send `Response::DispatchFailed`.
    pub fn call(&mut self, req: &Request) -> Result<Response, ClientError> {
        encode_frame(req, &mut self.stream)?;
        // Many UDS implementations keep writes unflushed in the kernel
        // buffer until a sufficient amount accumulates; flushing makes
        // tests deterministic regardless of buffer-pressure timing.
        self.stream.flush().map_err(FrameError::from)?;
        match decode_frame(&mut self.stream, self.max_frame_size)? {
            Some(resp) => Ok(resp),
            None => Err(ClientError::PeerClosed),
        }
    }

    /// Send a request without waiting for a response. Used for
    /// `Evict` when the substrate doesn't care about acknowledgement.
    pub fn send(&mut self, req: &Request) -> Result<(), ClientError> {
        encode_frame(req, &mut self.stream)?;
        self.stream.flush().map_err(FrameError::from)?;
        Ok(())
    }
}

impl std::fmt::Debug for WorkerRpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerRpcClient")
            .field("max_frame_size", &self.max_frame_size)
            .finish()
    }
}

/// Server-side helper: read one request from `stream`, decode, and
/// return it. Counterpart of [`WorkerRpcClient::call`]'s send half.
/// Used by tests and (in future commits) by the worker bootstrap.
pub fn server_recv_request(
    stream: &mut UnixStream,
    max_frame_size: usize,
) -> Result<Option<Request>, FrameError> {
    decode_frame(stream, max_frame_size)
}

/// Server-side helper: encode a response onto `stream`.
pub fn server_send_response(stream: &mut UnixStream, resp: &Response) -> Result<(), FrameError> {
    encode_frame(resp, stream)?;
    stream.flush()?;
    Ok(())
}

// Reads / Writes are passthroughs in case a caller needs raw access.
impl Read for WorkerRpcClient {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(buf)
    }
}

impl Write for WorkerRpcClient {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::protocol::{HealthInfo, NumericalMetadata, TargetKind};
    use std::thread;

    /// In-memory pair of UnixStreams. Used to drive the client on one
    /// thread and a synthetic worker on another.
    fn paired_streams() -> (UnixStream, UnixStream) {
        UnixStream::pair().expect("UnixStream::pair")
    }

    #[test]
    fn round_trip_health_call_over_uds_pair() {
        let (client_side, server_side) = paired_streams();

        let server = thread::spawn(move || {
            let mut server = server_side;
            // Read the request.
            let req = server_recv_request(&mut server, MAX_FRAME_SIZE_DEFAULT)
                .expect("server recv")
                .expect("request present");
            assert_eq!(req, Request::Health);
            // Send a synthetic response.
            let resp = Response::Health(HealthInfo {
                manifest_hash_in_image: Some("test-hash".to_string()),
                env_digest_in_image: None,
                numerical_metadata: NumericalMetadata {
                    blas_lib: Some("openblas".to_string()),
                    ..Default::default()
                },
            });
            server_send_response(&mut server, &resp).expect("server send");
        });

        let mut client = WorkerRpcClient::new(client_side);
        let resp = client.call(&Request::Health).expect("client call");
        match resp {
            Response::Health(info) => {
                assert_eq!(info.manifest_hash_in_image.as_deref(), Some("test-hash"));
                assert_eq!(
                    info.numerical_metadata.blas_lib.as_deref(),
                    Some("openblas")
                );
            }
            other => panic!("unexpected response: {other:?}"),
        }
        server.join().expect("server thread");
    }

    #[test]
    fn dispatch_method_round_trips_payload_bytes() {
        let (client_side, server_side) = paired_streams();
        let server = thread::spawn(move || {
            let mut server = server_side;
            let req = server_recv_request(&mut server, MAX_FRAME_SIZE_DEFAULT)
                .expect("server recv")
                .expect("request present");
            // Echo the inputs back as the output.
            let (invocation_id, target, inputs) = match req {
                Request::DispatchMethod {
                    invocation_id,
                    target_kind: _,
                    target,
                    inputs,
                } => (invocation_id, target, inputs),
                other => panic!("expected DispatchMethod, got {other:?}"),
            };
            assert_eq!(target.as_ref(), &[0xa0, 0x42, 0x01]);
            assert_eq!(inputs.len(), 2);
            let combined: Vec<u8> = inputs.iter().flat_map(|b| b.iter().copied()).collect();
            server_send_response(
                &mut server,
                &Response::DispatchOk {
                    invocation_id,
                    output: serde_bytes::ByteBuf::from(combined),
                    derivations: Vec::new(),
                    dispatched_to: Some("urn:eigenius:test:method:echo".to_string()),
                },
            )
            .expect("server send");
        });

        let mut client = WorkerRpcClient::new(client_side);
        let resp = client
            .call(&Request::DispatchMethod {
                invocation_id: "inv-7".to_string(),
                target_kind: TargetKind::Script,
                target: serde_bytes::ByteBuf::from(vec![0xa0, 0x42, 0x01]),
                inputs: vec![
                    serde_bytes::ByteBuf::from(vec![0x01, 0x02]),
                    serde_bytes::ByteBuf::from(vec![0x03]),
                ],
            })
            .expect("client call");
        match resp {
            Response::DispatchOk {
                invocation_id,
                output,
                derivations: _,
                dispatched_to,
            } => {
                assert_eq!(invocation_id, "inv-7");
                assert_eq!(output.as_ref(), &[0x01, 0x02, 0x03]);
                assert_eq!(
                    dispatched_to.as_deref(),
                    Some("urn:eigenius:test:method:echo")
                );
            }
            other => panic!("unexpected response: {other:?}"),
        }
        server.join().expect("server thread");
    }

    #[test]
    fn peer_closed_without_response_surfaces_distinctly() {
        let (client_side, server_side) = paired_streams();
        let server = thread::spawn(move || {
            let mut server = server_side;
            let _ = server_recv_request(&mut server, MAX_FRAME_SIZE_DEFAULT);
            // Drop without responding.
        });

        let mut client = WorkerRpcClient::new(client_side);
        let err = client
            .call(&Request::Health)
            .expect_err("call should fail when peer drops");
        assert!(matches!(err, ClientError::PeerClosed));
        server.join().expect("server thread");
    }

    #[test]
    fn send_does_not_wait_for_response() {
        let (client_side, server_side) = paired_streams();
        let server = thread::spawn(move || {
            let mut server = server_side;
            let req = server_recv_request(&mut server, MAX_FRAME_SIZE_DEFAULT)
                .expect("server recv")
                .expect("request present");
            assert_eq!(req, Request::Evict);
            // Don't send a response — `send` should not have waited
            // for one.
        });

        let mut client = WorkerRpcClient::new(client_side);
        client.send(&Request::Evict).expect("send");
        server.join().expect("server thread");
    }
}
