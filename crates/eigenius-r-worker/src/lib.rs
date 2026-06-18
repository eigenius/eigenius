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

//! `eigenius-r-worker` — worker-side substrate protocol shim for the R
//! language runtime (D55).
//!
//! Like [`eigenius-lean-worker`], this crate hosts the Unix-domain-socket
//! transport, the length-prefixed CBOR framing
//! ([`eigenius_runtime_substrate::rpc::codec`]), and the workspace
//! Eigon-CBOR codec — so the R side needs **no** native CBOR or socket
//! implementation. R loads the eventual cdylib via its C FFI (`.Call` /
//! `dyn.load`), drives the dispatch loop, and supplies only the
//! computation (limma / fgsea / lme4).
//!
//! ## Difference from the Lean worker
//!
//! The Lean worker handles only `TargetKind::Method` (typed multiple
//! dispatch) and rejects `Script`. R is the reverse: the WRN Tier-2
//! recomputes are `RunRuntimeScript` dispatches (an R source string over
//! chain-resident inputs), so the **`Script` path is first-class** here.
//! `Method` is still decoded for parity (the typed `call_method` path that
//! the P4 mirror lights up).
//!
//! ## P1 status (this module)
//!
//! The language-agnostic **protocol core** ([`RWorker`]) — listen / accept
//! / `next_request` / typed accessors / `send_*` — exercised in-process
//! over a [`UnixStream`] pair by the unit tests, no FFI boundary. The
//! extendr C-ABI surface (the cdylib R links) lands in P1 step 2; it will
//! be a thin `Raw`/`i32` wrapper over exactly these methods.

pub mod ffi;

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use eigenius_runtime_substrate::rpc::client::{server_recv_request, server_send_response};
use eigenius_runtime_substrate::rpc::codec::{FrameError, MAX_FRAME_SIZE_DEFAULT};
use eigenius_runtime_substrate::rpc::method::MethodInvocation;
use eigenius_runtime_substrate::rpc::protocol::{HealthInfo, Request, Response, TargetKind};

/// Discriminator returned by [`RWorker::next_request`]. The non-negative
/// variants name the in-flight request the accessors then read; the
/// negative variants are terminal (caller stops looping). The `i32` repr
/// is the wire the P1-step-2 extendr surface returns to R.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// Liveness + cross-check. Answer with [`RWorker::send_health`].
    Health = 0,
    /// Boot against the pinned environment. Answer with
    /// [`RWorker::send_instantiated`].
    Instantiate = 1,
    /// Load a package mirror. Answer with
    /// [`RWorker::send_mirror_registered`].
    RegisterMirror = 2,
    /// `DispatchMethod` with `target_kind = Script` — an R source string
    /// in [`RWorker::script_source`]. Answer with
    /// [`RWorker::send_dispatch_ok`] / [`RWorker::send_dispatch_failed`].
    DispatchScript = 3,
    /// `DispatchMethod` with `target_kind = Method` — a decoded
    /// [`MethodInvocation`] (the typed `call_method` path, P4).
    DispatchMethod = 4,
    /// Graceful shutdown. Answer with [`RWorker::send_evicted`].
    Evict = 5,

    /// Peer closed cleanly between frames. Stop looping; call drop.
    Closed = -1,
    /// Wire transport / CBOR decode failed (diagnostic on stderr).
    TransportError = -2,
    /// `DispatchMethod.target` could not be decoded for its `target_kind`
    /// (script source / `MethodInvocation`). Answer with
    /// [`RWorker::send_dispatch_failed`].
    MalformedDispatch = -3,
}

/// Pre-decoded form of the in-flight request. Accessors read out of it;
/// `send_*` consume it (set to `None`) once the response is dispatched.
enum InFlight {
    Health,
    Instantiate {
        env_iri: String,
        image_digest: Option<String>,
    },
    RegisterMirror {
        mirror_iri: String,
        library_content: Vec<u8>,
    },
    Script {
        invocation_id: String,
        source: String,
        inputs: Vec<Vec<u8>>,
    },
    Method {
        invocation_id: String,
        method: MethodInvocation,
        inputs: Vec<Vec<u8>>,
    },
    Evict,
}

/// Errors from the worker's send/receive helpers.
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("wire transport / CBOR framing failed: {0}")]
    Frame(#[from] FrameError),
    #[error("socket I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("no in-flight request to respond to")]
    NoInFlight,
    #[error("in-flight request is not a {expected} (cannot send {response})")]
    WrongVerb {
        expected: &'static str,
        response: &'static str,
    },
}

/// The worker's protocol state: an optional bound listener (Service-mode
/// lifecycle — one process accepts many substrate connections), the
/// current connection, and the decoded in-flight request.
pub struct RWorker {
    listener: Option<UnixListener>,
    stream: UnixStream,
    in_flight: Option<InFlight>,
}

impl RWorker {
    /// Wrap an already-connected stream. The path tests use (via
    /// [`UnixStream::pair`]); also the shape a pre-connected fd would take.
    pub fn from_stream(stream: UnixStream) -> Self {
        Self {
            listener: None,
            stream,
            in_flight: None,
        }
    }

    /// Bind a UDS listener at `path` (removing any stale socket first) and
    /// accept the first substrate connection.
    pub fn listen(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        let (stream, _addr) = listener.accept()?;
        Ok(Self {
            listener: Some(listener),
            stream,
            in_flight: None,
        })
    }

    /// Accept the next substrate connection on the bound listener,
    /// replacing the current stream. The substrate opens a fresh UDS
    /// connection per RPC (Health and DispatchMethod are separate dials).
    /// Errors if this worker was built via [`Self::from_stream`] (no
    /// listener).
    pub fn accept_next(&mut self) -> io::Result<()> {
        let listener = self.listener.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "accept_next on a listener-less worker (built from a stream pair)",
            )
        })?;
        let (stream, _addr) = listener.accept()?;
        self.stream = stream;
        self.in_flight = None;
        Ok(())
    }

    /// Read and decode the next request frame, storing it in-flight, and
    /// return its [`RequestKind`]. `Closed` on clean EOF; `TransportError`
    /// on a framing/decode failure; `MalformedDispatch` if a
    /// `DispatchMethod`'s `target` can't be decoded for its `target_kind`.
    pub fn next_request(&mut self) -> RequestKind {
        let req = match server_recv_request(&mut self.stream, MAX_FRAME_SIZE_DEFAULT) {
            Ok(Some(req)) => req,
            Ok(None) => return RequestKind::Closed,
            Err(e) => {
                eprintln!("eigenius-r-worker: bad request frame: {e}");
                return RequestKind::TransportError;
            }
        };
        let (kind, in_flight) = Self::decode_request(req);
        self.in_flight = Some(in_flight);
        kind
    }

    fn decode_request(req: Request) -> (RequestKind, InFlight) {
        match req {
            Request::Health => (RequestKind::Health, InFlight::Health),
            Request::Instantiate {
                env_iri,
                image_digest,
            } => (
                RequestKind::Instantiate,
                InFlight::Instantiate {
                    env_iri,
                    image_digest,
                },
            ),
            Request::RegisterMirror {
                mirror_iri,
                library_content,
            } => (
                RequestKind::RegisterMirror,
                InFlight::RegisterMirror {
                    mirror_iri,
                    library_content: library_content.into_vec(),
                },
            ),
            Request::DispatchMethod {
                invocation_id,
                target_kind,
                target,
                inputs,
            } => {
                let inputs: Vec<Vec<u8>> = inputs.into_iter().map(|b| b.into_vec()).collect();
                match target_kind {
                    TargetKind::Script => match ciborium::from_reader::<String, _>(&target[..]) {
                        Ok(source) => (
                            RequestKind::DispatchScript,
                            InFlight::Script {
                                invocation_id,
                                source,
                                inputs,
                            },
                        ),
                        Err(e) => {
                            eprintln!("eigenius-r-worker: malformed script source: {e}");
                            // Preserve the id so the caller can still send a
                            // correlated DispatchFailed.
                            (
                                RequestKind::MalformedDispatch,
                                InFlight::Script {
                                    invocation_id,
                                    source: String::new(),
                                    inputs,
                                },
                            )
                        }
                    },
                    TargetKind::Method => {
                        match ciborium::from_reader::<MethodInvocation, _>(&target[..]) {
                            Ok(method) => (
                                RequestKind::DispatchMethod,
                                InFlight::Method {
                                    invocation_id,
                                    method,
                                    inputs,
                                },
                            ),
                            Err(e) => {
                                eprintln!("eigenius-r-worker: malformed MethodInvocation: {e}");
                                (
                                    RequestKind::MalformedDispatch,
                                    InFlight::Method {
                                        invocation_id,
                                        method: MethodInvocation {
                                            function_name: String::new(),
                                            signature_iri: String::new(),
                                        },
                                        inputs,
                                    },
                                )
                            }
                        }
                    }
                }
            }
            Request::Evict => (RequestKind::Evict, InFlight::Evict),
        }
    }

    // ── Accessors over the in-flight request ────────────────────────────

    /// Correlation id of the in-flight dispatch (`Script` / `Method`).
    pub fn invocation_id(&self) -> Option<&str> {
        match &self.in_flight {
            Some(InFlight::Script { invocation_id, .. })
            | Some(InFlight::Method { invocation_id, .. }) => Some(invocation_id),
            _ => None,
        }
    }

    /// The R source for an in-flight `DispatchScript`.
    pub fn script_source(&self) -> Option<&str> {
        match &self.in_flight {
            Some(InFlight::Script { source, .. }) => Some(source),
            _ => None,
        }
    }

    /// The function name for an in-flight `DispatchMethod`.
    pub fn function_name(&self) -> Option<&str> {
        match &self.in_flight {
            Some(InFlight::Method { method, .. }) => Some(&method.function_name),
            _ => None,
        }
    }

    /// The signature IRI for an in-flight `DispatchMethod`.
    pub fn signature_iri(&self) -> Option<&str> {
        match &self.in_flight {
            Some(InFlight::Method { method, .. }) => Some(&method.signature_iri),
            _ => None,
        }
    }

    /// CBOR-encoded input resources of the in-flight dispatch, in argument
    /// order. Empty for non-dispatch requests.
    pub fn inputs(&self) -> &[Vec<u8>] {
        match &self.in_flight {
            Some(InFlight::Script { inputs, .. }) | Some(InFlight::Method { inputs, .. }) => inputs,
            _ => &[],
        }
    }

    /// The env IRI of an in-flight `Instantiate`.
    pub fn env_iri(&self) -> Option<&str> {
        match &self.in_flight {
            Some(InFlight::Instantiate { env_iri, .. }) => Some(env_iri),
            _ => None,
        }
    }

    /// The image digest of an in-flight `Instantiate` (`None` under
    /// LocalSpawner).
    pub fn image_digest(&self) -> Option<&str> {
        match &self.in_flight {
            Some(InFlight::Instantiate { image_digest, .. }) => image_digest.as_deref(),
            _ => None,
        }
    }

    /// The mirror IRI of an in-flight `RegisterMirror`.
    pub fn mirror_iri(&self) -> Option<&str> {
        match &self.in_flight {
            Some(InFlight::RegisterMirror { mirror_iri, .. }) => Some(mirror_iri),
            _ => None,
        }
    }

    /// The mirror archive bytes of an in-flight `RegisterMirror`.
    pub fn library_content(&self) -> Option<&[u8]> {
        match &self.in_flight {
            Some(InFlight::RegisterMirror {
                library_content, ..
            }) => Some(library_content),
            _ => None,
        }
    }

    // ── Response senders (consume the in-flight request) ────────────────

    /// Reply to a `Health` request.
    pub fn send_health(&mut self, info: HealthInfo) -> Result<(), WorkerError> {
        self.respond(Response::Health(info))
    }

    /// Reply to an `Instantiate` request.
    pub fn send_instantiated(&mut self, ready: bool) -> Result<(), WorkerError> {
        self.respond(Response::Instantiated { ready })
    }

    /// Reply to a `RegisterMirror` request (echoes the mirror IRI).
    pub fn send_mirror_registered(&mut self) -> Result<(), WorkerError> {
        let mirror_iri = match &self.in_flight {
            Some(InFlight::RegisterMirror { mirror_iri, .. }) => mirror_iri.clone(),
            _ => {
                return Err(WorkerError::WrongVerb {
                    expected: "RegisterMirror",
                    response: "MirrorRegistered",
                })
            }
        };
        self.respond(Response::MirrorRegistered { mirror_iri })
    }

    /// Reply to a `DispatchScript` / `DispatchMethod` with a successful
    /// result. `output` is a CBOR-encoded Eigon resource (the
    /// `DerivedResource` R produced); `derivations` are side-effect
    /// resources; `dispatched_to` is the resolved signature IRI for the
    /// typed-method path (`None` for scripts). Echoes the invocation id.
    pub fn send_dispatch_ok(
        &mut self,
        output: Vec<u8>,
        derivations: Vec<Vec<u8>>,
        dispatched_to: Option<String>,
    ) -> Result<(), WorkerError> {
        let invocation_id = self.dispatch_invocation_id("DispatchOk")?;
        self.respond(Response::DispatchOk {
            invocation_id,
            output: output.into(),
            derivations: derivations.into_iter().map(Into::into).collect(),
            dispatched_to,
        })
    }

    /// Reply to a dispatch with a failure. `error_kind` maps to a
    /// `RunError` variant; `message` carries the R-side diagnostic.
    pub fn send_dispatch_failed(
        &mut self,
        error_kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<(), WorkerError> {
        let invocation_id = self.dispatch_invocation_id("DispatchFailed")?;
        self.respond(Response::DispatchFailed {
            invocation_id,
            error_kind: error_kind.into(),
            message: message.into(),
        })
    }

    /// Reply to an `Evict` request (sent immediately before exit).
    pub fn send_evicted(&mut self) -> Result<(), WorkerError> {
        self.respond(Response::Evicted)
    }

    fn dispatch_invocation_id(&self, response: &'static str) -> Result<String, WorkerError> {
        match &self.in_flight {
            Some(InFlight::Script { invocation_id, .. })
            | Some(InFlight::Method { invocation_id, .. }) => Ok(invocation_id.clone()),
            Some(_) => Err(WorkerError::WrongVerb {
                expected: "DispatchScript/DispatchMethod",
                response,
            }),
            None => Err(WorkerError::NoInFlight),
        }
    }

    fn respond(&mut self, response: Response) -> Result<(), WorkerError> {
        if self.in_flight.is_none() {
            return Err(WorkerError::NoInFlight);
        }
        server_send_response(&mut self.stream, &response)?;
        self.in_flight = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_runtime_substrate::rpc::codec::{decode_frame, encode_frame};
    use eigenius_runtime_substrate::rpc::protocol::NumericalMetadata;
    use serde_bytes::ByteBuf;
    use std::thread;

    /// Drive the worker over an in-process `UnixStream` pair (no FFI, no
    /// real listener) — the P1 milestone, mirroring the Lean worker's
    /// `uds_round_trip`. The "substrate" side runs on a thread sending
    /// requests and reading responses; the worker side runs on the main
    /// thread via the [`RWorker`] API.
    #[test]
    fn health_and_script_round_trip() {
        let (client, server) = UnixStream::pair().expect("socketpair");

        let client_thread = thread::spawn(move || {
            let mut client = client;
            // 1. Health.
            encode_frame(&Request::Health, &mut client).expect("send health");
            let resp: Response = decode_frame(&mut client, MAX_FRAME_SIZE_DEFAULT)
                .expect("recv health")
                .expect("health frame");
            assert!(matches!(resp, Response::Health(_)), "got {resp:?}");

            // 2. DispatchScript with one input resource.
            let mut source_cbor = Vec::new();
            ciborium::into_writer(&"limma::topTable(fit)".to_string(), &mut source_cbor).unwrap();
            let req = Request::DispatchMethod {
                invocation_id: "inv-1".to_string(),
                target_kind: TargetKind::Script,
                target: ByteBuf::from(source_cbor),
                inputs: vec![ByteBuf::from(vec![0xDE, 0xAD, 0xBE, 0xEF])],
            };
            encode_frame(&req, &mut client).expect("send script");
            let resp: Response = decode_frame(&mut client, MAX_FRAME_SIZE_DEFAULT)
                .expect("recv dispatch")
                .expect("dispatch frame");
            match resp {
                Response::DispatchOk {
                    invocation_id,
                    output,
                    dispatched_to,
                    ..
                } => {
                    assert_eq!(invocation_id, "inv-1");
                    assert_eq!(&output[..], b"RESULT-CBOR");
                    assert_eq!(dispatched_to, None);
                }
                other => panic!("expected DispatchOk, got {other:?}"),
            }

            // 3. Evict.
            encode_frame(&Request::Evict, &mut client).expect("send evict");
            let resp: Response = decode_frame(&mut client, MAX_FRAME_SIZE_DEFAULT)
                .expect("recv evicted")
                .expect("evicted frame");
            assert!(matches!(resp, Response::Evicted), "got {resp:?}");

            // 4. Clean close.
            drop(client);
        });

        let mut worker = RWorker::from_stream(server);

        // 1. Health.
        assert_eq!(worker.next_request(), RequestKind::Health);
        worker
            .send_health(HealthInfo {
                manifest_hash_in_image: None,
                env_digest_in_image: None,
                numerical_metadata: NumericalMetadata::default(),
            })
            .expect("send health");

        // 2. Script — read the source + input, answer DispatchOk.
        assert_eq!(worker.next_request(), RequestKind::DispatchScript);
        assert_eq!(worker.invocation_id(), Some("inv-1"));
        assert_eq!(worker.script_source(), Some("limma::topTable(fit)"));
        assert_eq!(worker.inputs().len(), 1);
        assert_eq!(worker.inputs()[0], vec![0xDE, 0xAD, 0xBE, 0xEF]);
        worker
            .send_dispatch_ok(b"RESULT-CBOR".to_vec(), vec![], None)
            .expect("send ok");

        // 3. Evict.
        assert_eq!(worker.next_request(), RequestKind::Evict);
        worker.send_evicted().expect("send evicted");

        // 4. Peer closed.
        assert_eq!(worker.next_request(), RequestKind::Closed);

        client_thread.join().expect("client thread");
    }

    #[test]
    fn malformed_script_target_is_reported_with_correlated_failure() {
        let (client, server) = UnixStream::pair().expect("socketpair");
        let client_thread = thread::spawn(move || {
            let mut client = client;
            // `target` is not a CBOR string — a CBOR array, say.
            let mut bad = Vec::new();
            ciborium::into_writer(&vec![1u8, 2, 3], &mut bad).unwrap();
            let req = Request::DispatchMethod {
                invocation_id: "inv-bad".to_string(),
                target_kind: TargetKind::Script,
                target: ByteBuf::from(bad),
                inputs: vec![],
            };
            encode_frame(&req, &mut client).expect("send bad script");
            let resp: Response = decode_frame(&mut client, MAX_FRAME_SIZE_DEFAULT)
                .expect("recv")
                .expect("frame");
            match resp {
                Response::DispatchFailed {
                    invocation_id,
                    error_kind,
                    ..
                } => {
                    assert_eq!(invocation_id, "inv-bad");
                    assert_eq!(error_kind, "malformed_dispatch");
                }
                other => panic!("expected DispatchFailed, got {other:?}"),
            }
        });

        let mut worker = RWorker::from_stream(server);
        assert_eq!(worker.next_request(), RequestKind::MalformedDispatch);
        // The id is preserved so the failure is correlated.
        assert_eq!(worker.invocation_id(), Some("inv-bad"));
        worker
            .send_dispatch_failed("malformed_dispatch", "target is not a CBOR string")
            .expect("send failure");
        client_thread.join().expect("client thread");
    }
}
