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

//! `eigenius-lean-worker` — Rust ↔ Lean FFI shim for the Lean
//! language runtime's worker process.
//!
//! Compiles to a `cdylib` the Lake-built Lean worker links against
//! via `lakefile.lean`'s `extraLinkArgs`. Rust handles transport
//! (UDS + CBOR framing per [D26](../../docs/design/d26-runtime-substrate.md)
//! §8.1) and CBOR codec work; Lean drives the main loop, dispatches
//! on a small set of scalar-typed verb tags, and assembles responses
//! by calling back into Rust per-verb send helpers.
//!
//! ## Why polling, not callbacks
//!
//! Lean 4's `@[export]` directive only handles POD-shaped return
//! types (scalars, `ByteArray`, `String`) — returning a multi-field
//! C struct by value across the FFI boundary is fragile and would
//! force the Lean side either to link Lean's own runtime API into
//! Rust or to use out-parameters that don't fit `IO`'s monadic
//! shape. Inverting the loop fixes both: Rust exposes primitives
//! returning POD types, Lean drives the main loop in idiomatic
//! `IO`, and the only Lean-side type that crosses the boundary is
//! `ByteArray` (wrapping the worker's [`OwnedBytes`]).
//!
//! ## State machine
//!
//! A [`WorkerHandle`] holds the accepted [`UnixStream`] plus an
//! optional in-flight request. Caller usage cycles through:
//!
//! 1. [`worker_listen`] — bind a UDS, accept one substrate
//!    connection, return a handle.
//! 2. [`worker_next_request_kind`] — block until a request frame
//!    arrives, decode it into the handle's in-flight slot, return
//!    the verb-discriminator integer.
//! 3. Field accessors (e.g. [`worker_request_invocation_id`],
//!    [`worker_request_function_name`], [`worker_request_input`])
//!    — read fields out of the in-flight request as
//!    [`OwnedBytes`] the caller frees via
//!    [`worker_free_owned_bytes`].
//! 4. A per-verb send helper (e.g. [`worker_send_dispatch_ok`]) —
//!    serialise the response, write it to the stream, clear the
//!    in-flight request.
//! 5. Loop back to step 2 (or [`worker_close`] to terminate).
//!
//! Calling an accessor when there's no in-flight request, or
//! sending a response of the wrong verb, returns the empty
//! [`OwnedBytes`] / an [`ExitCode`] error — the worker doesn't
//! panic, so a buggy Lean caller surfaces as a clean diagnostic.
//!
//! ## Ownership of byte buffers
//!
//! Two C-ABI types carry buffers across the FFI boundary:
//!
//! - [`Bytes`] — borrowed view. Producer (the Lean caller passing
//!   send payloads in) retains ownership; consumer (Rust) reads
//!   without freeing.
//! - [`OwnedBytes`] — ownership transferred. Producer (Rust
//!   returning a field from an in-flight request) hands a
//!   heap-allocated buffer to the consumer (Lean), which must
//!   release it via [`worker_free_owned_bytes`].

#![allow(clippy::missing_safety_doc)]

pub mod lean_ffi;
pub mod lean_project;
pub mod lean_sys;

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::slice;

use eigenius_runtime_substrate::rpc::client::{server_recv_request, server_send_response};
use eigenius_runtime_substrate::rpc::codec::MAX_FRAME_SIZE_DEFAULT;
use eigenius_runtime_substrate::rpc::protocol::{
    HealthInfo, NumericalMetadata, Request, Response, TargetKind,
};

// ---------------------------------------------------------------------------
// C-ABI types
// ---------------------------------------------------------------------------

/// Borrowed view of a byte buffer. Producer retains ownership;
/// consumer reads `len` bytes starting at `ptr` and must not free.
///
/// Used Lean→Rust for send-helper payloads (e.g. the response bytes
/// the Lean handler returns for `worker_send_dispatch_ok`). Lean
/// owns the underlying `ByteArray`; Rust copies the bytes into its
/// own buffer during the call.
#[repr(C)]
pub struct Bytes {
    pub ptr: *const u8,
    pub len: usize,
}

impl Bytes {
    /// Construct a borrowed view from a `&[u8]`. The lifetime of the
    /// resulting `Bytes` is implicit — caller must ensure the
    /// underlying slice outlives any use of the `Bytes` value.
    pub fn from_slice(s: &[u8]) -> Self {
        Self {
            ptr: s.as_ptr(),
            len: s.len(),
        }
    }

    /// Returns the view's bytes as a `&[u8]`. The result borrows the
    /// underlying memory for the duration of `self`. Returns an
    /// empty slice if `ptr` is null.
    ///
    /// # Safety
    /// `ptr` must be valid for reads of `len` bytes or be null.
    pub unsafe fn as_slice(&self) -> &[u8] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

/// Owned byte buffer transferred Rust→Lean. Lean must release via
/// [`worker_free_owned_bytes`].
///
/// `ptr` + `len` describe the populated portion; `cap` carries the
/// allocator's capacity so the free function knows the underlying
/// allocation size.
#[repr(C)]
#[derive(Debug)]
pub struct OwnedBytes {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl OwnedBytes {
    /// Empty buffer sentinel — `ptr` is null, `len`/`cap` zero.
    /// [`worker_free_owned_bytes`] is a no-op on this value, so
    /// callers can freely use it as a "field not present" marker.
    pub const fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }

    /// True if the buffer is the empty sentinel.
    pub fn is_empty(&self) -> bool {
        self.ptr.is_null() && self.len == 0
    }

    /// Construct from a `Vec<u8>` by consuming it. Used when Rust
    /// hands ownership of a decoded request field to Lean.
    pub fn from_vec(v: Vec<u8>) -> Self {
        if v.is_empty() {
            return Self::empty();
        }
        let mut v = std::mem::ManuallyDrop::new(v);
        Self {
            ptr: v.as_mut_ptr(),
            len: v.len(),
            cap: v.capacity(),
        }
    }

    /// Construct from a `String` (UTF-8 bytes).
    pub fn from_string(s: String) -> Self {
        Self::from_vec(s.into_bytes())
    }
}

/// Verb discriminator returned by [`worker_next_request_kind`].
/// Negative values reserved for protocol-level errors (the Lean
/// side exits the main loop on any negative kind).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Health = 0,
    Instantiate = 1,
    RegisterMirror = 2,
    DispatchMethod = 3,
    Evict = 4,

    /// Peer closed cleanly between frames. Caller should stop
    /// looping and call [`worker_close`].
    Closed = -1,
    /// Wire transport / CBOR decode failed. The worker logs the
    /// diagnostic to stderr; caller should stop looping.
    TransportError = -2,
    /// `target_kind = Script` on `DispatchMethod` — Lean worker
    /// doesn't host script-eval. Caller should send a
    /// `DispatchFailed` response via
    /// [`worker_send_dispatch_failed`].
    UnsupportedScriptKind = -3,
    /// `MethodInvocation` decode from `DispatchMethod.target`
    /// failed. Caller should send a `DispatchFailed` response.
    MalformedMethodInvocation = -4,
}

/// Exit codes returned by [`worker_send_*`] helpers and
/// [`worker_listen`]. Zero = success.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Ok = 0,
    BadUdsPath = 1,
    BindFailed = 2,
    AcceptFailed = 3,
    WriteFailed = 4,
    NoInFlightRequest = 5,
    WrongResponseVerb = 6,
}

// ---------------------------------------------------------------------------
// WorkerHandle — the polling-API state object
// ---------------------------------------------------------------------------

/// Internal handle state. Allocated by [`worker_listen`], handed
/// to Lean as an opaque `*mut WorkerHandle`, freed by
/// [`worker_close`].
///
/// `Box`-allocated so its address is stable across calls — Lean
/// keeps the same pointer across many `worker_next_request_kind` /
/// `worker_send_*` round-trips.
pub struct WorkerHandle {
    /// The bound listener — kept on the handle so a single worker
    /// process can accept multiple substrate-side connections
    /// across its lifetime (D26 §8.1 Service-mode lifecycle). The
    /// substrate opens a fresh UDS connection per RPC (Health and
    /// DispatchMethod are separate dials); each accept on the
    /// listener becomes the new value of `stream`. `None` in unit
    /// tests where the stream is constructed from an in-process
    /// `UnixStream::pair` and no listener exists.
    listener: Option<UnixListener>,
    stream: UnixStream,
    in_flight: Option<InFlightRequest>,
}

/// Pre-decoded representation of the currently in-flight request.
/// Field accessors read out of this; send helpers consume it (set
/// to `None`) after dispatching the response.
enum InFlightRequest {
    Health,
    Instantiate {
        env_iri: String,
        image_digest: Option<String>,
    },
    RegisterMirror {
        mirror_iri: String,
        library_content: Vec<u8>,
    },
    DispatchMethod {
        invocation_id: String,
        function_name: String,
        signature_iri: String,
        inputs: Vec<Vec<u8>>,
    },
    Evict,
}

// ---------------------------------------------------------------------------
// C-ABI entry points
// ---------------------------------------------------------------------------

/// Bind a UDS listener at `uds_path` (a UTF-8 byte slice — *not*
/// null-terminated), accept one substrate connection, and return
/// an opaque handle. `null` on failure (diagnostic on stderr).
///
/// Removes any stale file at the path before binding so a previous
/// worker's leftover socket doesn't fail-fast the new bind.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_listen(
    uds_path_ptr: *const u8,
    uds_path_len: usize,
) -> *mut WorkerHandle {
    let path_slice = unsafe { slice::from_raw_parts(uds_path_ptr, uds_path_len) };
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("eigenius-lean-worker: uds_path is not valid UTF-8");
            return std::ptr::null_mut();
        }
    };
    let path = Path::new(path_str);

    // Tolerate a leftover socket file from a prior worker.
    let _ = std::fs::remove_file(path);

    let listener = match UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("eigenius-lean-worker: failed to bind UDS at `{path_str}`: {e}");
            return std::ptr::null_mut();
        }
    };

    // Open up the socket so a non-root host process can connect when
    // the worker runs as root inside a container with the tempdir
    // bind-mounted. UnixListener::bind defaults to mode 0o755 (no
    // world-write), which blocks the substrate's client-side connect
    // with `Permission denied`. Matches JuliaWorker.jl's
    // `chmod(uds_path, 0o666)` after `listen`.
    #[cfg(unix)]
    if let Err(e) = std::fs::set_permissions(
        path,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o666),
    ) {
        eprintln!("eigenius-lean-worker: failed to chmod 0o666 on `{path_str}`: {e}");
        return std::ptr::null_mut();
    }

    let (stream, _addr) = match listener.accept() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("eigenius-lean-worker: failed to accept on `{path_str}`: {e}");
            return std::ptr::null_mut();
        }
    };

    let handle = Box::new(WorkerHandle {
        listener: Some(listener),
        stream,
        in_flight: None,
    });
    Box::into_raw(handle)
}

/// Drop the current `stream` and accept the next substrate-side
/// connection on the bound listener. Lean's `runLoop` invokes this
/// when the peer closes a connection (substrate uses one connection
/// per RPC) so the worker stays alive for the next dispatch instead
/// of exiting on every `Health` or `DispatchMethod` round-trip.
/// Returns `0` on success, non-zero on accept failure.
///
/// # Safety
///
/// `handle` must point at a live [`WorkerHandle`] previously returned
/// by [`worker_listen`] and not yet freed via [`worker_close`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_accept_next(handle: *mut WorkerHandle) -> i32 {
    if handle.is_null() {
        return 1;
    }
    let h = unsafe { &mut *handle };
    let Some(listener) = h.listener.as_ref() else {
        eprintln!("eigenius-lean-worker: accept_next called on handle without listener");
        return 3;
    };
    // Clear any in-flight slot from the previous connection so a
    // request reader on the new stream doesn't see stale state.
    h.in_flight = None;
    match listener.accept() {
        Ok((stream, _addr)) => {
            h.stream = stream;
            0
        }
        Err(e) => {
            eprintln!("eigenius-lean-worker: accept_next failed: {e}");
            2
        }
    }
}

/// Free a [`WorkerHandle`]. Drops the underlying [`UnixStream`]
/// (closing the connection). Safe to call with a null pointer
/// (no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_close(h: *mut WorkerHandle) {
    if h.is_null() {
        return;
    }
    let _ = unsafe { Box::from_raw(h) };
}

/// Block until the next request arrives, decode it into the
/// handle's in-flight slot, return the verb discriminator (a
/// [`RequestKind`] value as `i32`).
///
/// After a successful return, the caller reads request fields via
/// the field-accessor helpers, then dispatches a response via the
/// matching `worker_send_*` helper. The accessors / senders all
/// reference the in-flight slot — calling
/// `worker_next_request_kind` again before sending a response
/// overwrites the slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_next_request_kind(h: *mut WorkerHandle) -> i32 {
    let handle = match unsafe { h.as_mut() } {
        Some(h) => h,
        None => return RequestKind::TransportError as i32,
    };

    let req = match server_recv_request(&mut handle.stream, MAX_FRAME_SIZE_DEFAULT) {
        Ok(Some(r)) => r,
        Ok(None) => return RequestKind::Closed as i32,
        Err(e) => {
            eprintln!("eigenius-lean-worker: bad request frame: {e}");
            return RequestKind::TransportError as i32;
        }
    };

    let (kind, in_flight) = decode_request(req);
    handle.in_flight = Some(in_flight);
    kind as i32
}

/// Map a substrate `Request` into the worker's in-flight enum +
/// the `RequestKind` discriminator. Pulls the `MethodInvocation`
/// decode for `DispatchMethod` out of the Lean caller's hands —
/// they get a plain `function_name` string back.
fn decode_request(req: Request) -> (RequestKind, InFlightRequest) {
    match req {
        Request::Health => (RequestKind::Health, InFlightRequest::Health),

        Request::Instantiate {
            env_iri,
            image_digest,
        } => (
            RequestKind::Instantiate,
            InFlightRequest::Instantiate {
                env_iri,
                image_digest,
            },
        ),

        Request::RegisterMirror {
            mirror_iri,
            library_content,
        } => (
            RequestKind::RegisterMirror,
            InFlightRequest::RegisterMirror {
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
            if target_kind != TargetKind::Method {
                // Surface as a typed kind so the Lean side knows to
                // send a DispatchFailed without bothering with field
                // accessors. We still hold the invocation_id we
                // need to respond — stash it in the in-flight slot
                // so worker_send_dispatch_failed can build the
                // response.
                let in_flight = InFlightRequest::DispatchMethod {
                    invocation_id,
                    function_name: String::new(),
                    signature_iri: String::new(),
                    inputs: inputs.into_iter().map(|b| b.into_vec()).collect(),
                };
                return (RequestKind::UnsupportedScriptKind, in_flight);
            }

            let invocation: eigenius_runtime_substrate::rpc::method::MethodInvocation =
                match ciborium::from_reader(target.as_ref()) {
                    Ok(mi) => mi,
                    Err(_) => {
                        let in_flight = InFlightRequest::DispatchMethod {
                            invocation_id,
                            function_name: String::new(),
                            signature_iri: String::new(),
                            inputs: inputs.into_iter().map(|b| b.into_vec()).collect(),
                        };
                        return (RequestKind::MalformedMethodInvocation, in_flight);
                    }
                };

            (
                RequestKind::DispatchMethod,
                InFlightRequest::DispatchMethod {
                    invocation_id,
                    function_name: invocation.function_name,
                    signature_iri: invocation.signature_iri,
                    inputs: inputs.into_iter().map(|b| b.into_vec()).collect(),
                },
            )
        }

        Request::Evict => (RequestKind::Evict, InFlightRequest::Evict),
    }
}

// ---------------------------------------------------------------------------
// Field accessors. Read out of the in-flight slot. Each returns
// `OwnedBytes::empty()` if no in-flight request or the field
// doesn't apply to the current verb.
// ---------------------------------------------------------------------------

/// Read the `env_iri` from an in-flight Instantiate request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_request_env_iri(h: *mut WorkerHandle) -> OwnedBytes {
    let handle = match unsafe { h.as_ref() } {
        Some(h) => h,
        None => return OwnedBytes::empty(),
    };
    match &handle.in_flight {
        Some(InFlightRequest::Instantiate { env_iri, .. }) => {
            OwnedBytes::from_string(env_iri.clone())
        }
        _ => OwnedBytes::empty(),
    }
}

/// Read the `image_digest` from an in-flight Instantiate request.
/// Empty if the substrate didn't supply one (LocalSpawner mode).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_request_image_digest(h: *mut WorkerHandle) -> OwnedBytes {
    let handle = match unsafe { h.as_ref() } {
        Some(h) => h,
        None => return OwnedBytes::empty(),
    };
    match &handle.in_flight {
        Some(InFlightRequest::Instantiate { image_digest, .. }) => image_digest
            .as_ref()
            .map(|s| OwnedBytes::from_string(s.clone()))
            .unwrap_or_else(OwnedBytes::empty),
        _ => OwnedBytes::empty(),
    }
}

/// Read the `mirror_iri` from an in-flight RegisterMirror request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_request_mirror_iri(h: *mut WorkerHandle) -> OwnedBytes {
    let handle = match unsafe { h.as_ref() } {
        Some(h) => h,
        None => return OwnedBytes::empty(),
    };
    match &handle.in_flight {
        Some(InFlightRequest::RegisterMirror { mirror_iri, .. }) => {
            OwnedBytes::from_string(mirror_iri.clone())
        }
        _ => OwnedBytes::empty(),
    }
}

/// Read the `library_content` archive bytes from an in-flight
/// RegisterMirror request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_request_library_content(h: *mut WorkerHandle) -> OwnedBytes {
    let handle = match unsafe { h.as_ref() } {
        Some(h) => h,
        None => return OwnedBytes::empty(),
    };
    match &handle.in_flight {
        Some(InFlightRequest::RegisterMirror {
            library_content, ..
        }) => OwnedBytes::from_vec(library_content.clone()),
        _ => OwnedBytes::empty(),
    }
}

/// Read the `invocation_id` from an in-flight DispatchMethod
/// request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_request_invocation_id(h: *mut WorkerHandle) -> OwnedBytes {
    let handle = match unsafe { h.as_ref() } {
        Some(h) => h,
        None => return OwnedBytes::empty(),
    };
    match &handle.in_flight {
        Some(InFlightRequest::DispatchMethod { invocation_id, .. }) => {
            OwnedBytes::from_string(invocation_id.clone())
        }
        _ => OwnedBytes::empty(),
    }
}

/// Read the `function_name` from an in-flight DispatchMethod
/// request — the `lean_export` vs user-method discriminator the
/// Lean handler dispatches on.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_request_function_name(h: *mut WorkerHandle) -> OwnedBytes {
    let handle = match unsafe { h.as_ref() } {
        Some(h) => h,
        None => return OwnedBytes::empty(),
    };
    match &handle.in_flight {
        Some(InFlightRequest::DispatchMethod { function_name, .. }) => {
            OwnedBytes::from_string(function_name.clone())
        }
        _ => OwnedBytes::empty(),
    }
}

/// Read the `signature_iri` from an in-flight DispatchMethod
/// request — the RuntimeMethodSignature IRI the worker should
/// echo on `DispatchOk.dispatched_to`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_request_signature_iri(h: *mut WorkerHandle) -> OwnedBytes {
    let handle = match unsafe { h.as_ref() } {
        Some(h) => h,
        None => return OwnedBytes::empty(),
    };
    match &handle.in_flight {
        Some(InFlightRequest::DispatchMethod { signature_iri, .. }) => {
            OwnedBytes::from_string(signature_iri.clone())
        }
        _ => OwnedBytes::empty(),
    }
}

/// Number of positional inputs on an in-flight DispatchMethod
/// request. Zero for non-DispatchMethod / no in-flight request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_request_input_count(h: *mut WorkerHandle) -> usize {
    let handle = match unsafe { h.as_ref() } {
        Some(h) => h,
        None => return 0,
    };
    match &handle.in_flight {
        Some(InFlightRequest::DispatchMethod { inputs, .. }) => inputs.len(),
        _ => 0,
    }
}

/// Read one positional input by index. Empty if out of range / no
/// in-flight DispatchMethod.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_request_input(h: *mut WorkerHandle, index: usize) -> OwnedBytes {
    let handle = match unsafe { h.as_ref() } {
        Some(h) => h,
        None => return OwnedBytes::empty(),
    };
    match &handle.in_flight {
        Some(InFlightRequest::DispatchMethod { inputs, .. }) => inputs
            .get(index)
            .map(|v| OwnedBytes::from_vec(v.clone()))
            .unwrap_or_else(OwnedBytes::empty),
        _ => OwnedBytes::empty(),
    }
}

// ---------------------------------------------------------------------------
// Eigon-CBOR decoders exposed to Lean.
//
// The cdylib hosts the workspace's Eigon-CBOR codec so the Lake worker
// can read structured Eigon Resources off the wire without teaching
// Lean a parallel CBOR implementation. The substrate ships every
// `call_method` input as `eigon_cbor::serialize_resource(...)`; the
// worker uses these helpers to pull individual property values out of
// the resulting bytes inside its dispatch handlers.
// ---------------------------------------------------------------------------

/// Parse `cbor` as an Eigon Resource and return the UTF-8 bytes of its
/// `property_iri` string property. Returns an empty `OwnedBytes` when:
///
/// - the bytes don't decode as a Resource (lenient parse path),
/// - `property_iri` is malformed,
/// - the property is absent on the resource, or
/// - the property's value isn't a `Value::String` (numbers, arrays,
///   nested resources, etc. all surface as empty).
///
/// The empty-on-failure shape matches the existing accessor pattern
/// (`worker_request_*`) — callers inspect the returned ByteArray's
/// size and dispatch a `DispatchFailed` themselves if the value is
/// missing. The cdylib stays decoupled from the worker's error
/// surface this way.
///
/// # Safety
///
/// Standard FFI contract: both `cbor` and `property_iri` must point
/// at memory the caller owns for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_decode_eigon_string_property(
    cbor: Bytes,
    property_iri: Bytes,
) -> OwnedBytes {
    let cbor_bytes = unsafe { cbor.as_slice() };
    let iri_bytes = unsafe { property_iri.as_slice() };
    decode_eigon_string_property(cbor_bytes, iri_bytes)
        .map(OwnedBytes::from_vec)
        .unwrap_or_else(OwnedBytes::empty)
}

fn decode_eigon_string_property(cbor: &[u8], property_iri: &[u8]) -> Option<Vec<u8>> {
    let iri_str = std::str::from_utf8(property_iri).ok()?;
    let iri = eigenius_kernel::ontology::iri::Iri::parse(iri_str).ok()?;
    let resource = eigenius_kernel::ontology::eigon_cbor::parse_resource_lenient(cbor).ok()?;
    let value = resource.get(&iri)?;
    let s = value.as_str()?;
    Some(s.as_bytes().to_vec())
}

// ---------------------------------------------------------------------------
// Send helpers. Each builds the appropriate Response, writes the
// frame, and clears the in-flight slot.
// ---------------------------------------------------------------------------

/// Send a [`Response::Health`] with worker self-reported
/// metadata. Populates `manifest_hash_in_image` / `env_digest_in_image`
/// from env vars the substrate sets (D26 §9.3); both `None` in a
/// LocalSpawner / dev shell.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_send_health(h: *mut WorkerHandle) -> i32 {
    let handle = match unsafe { h.as_mut() } {
        Some(h) => h,
        None => return ExitCode::NoInFlightRequest as i32,
    };
    let response = Response::Health(default_health_info());
    send_response(handle, response, RequestKindMatch::Any)
}

/// Send a [`Response::Instantiated`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_send_instantiated(h: *mut WorkerHandle, ready: bool) -> i32 {
    let handle = match unsafe { h.as_mut() } {
        Some(h) => h,
        None => return ExitCode::NoInFlightRequest as i32,
    };
    let response = Response::Instantiated { ready };
    send_response(handle, response, RequestKindMatch::Any)
}

/// Send a [`Response::MirrorRegistered`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_send_mirror_registered(
    h: *mut WorkerHandle,
    mirror_iri: Bytes,
) -> i32 {
    let handle = match unsafe { h.as_mut() } {
        Some(h) => h,
        None => return ExitCode::NoInFlightRequest as i32,
    };
    let iri = match std::str::from_utf8(unsafe { mirror_iri.as_slice() }) {
        Ok(s) => s.to_string(),
        Err(_) => return ExitCode::WriteFailed as i32,
    };
    let response = Response::MirrorRegistered { mirror_iri: iri };
    send_response(handle, response, RequestKindMatch::Any)
}

/// Send a [`Response::DispatchOk`]. `output` carries the response
/// bytes; `dispatched_to` carries the RuntimeMethodSignature IRI
/// (empty `Bytes` → falls back to the in-flight request's
/// `signature_iri`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_send_dispatch_ok(
    h: *mut WorkerHandle,
    output: Bytes,
    dispatched_to: Bytes,
) -> i32 {
    let handle = match unsafe { h.as_mut() } {
        Some(h) => h,
        None => return ExitCode::NoInFlightRequest as i32,
    };

    let (invocation_id, fallback_dispatched_to) = match &handle.in_flight {
        Some(InFlightRequest::DispatchMethod {
            invocation_id,
            signature_iri,
            ..
        }) => (invocation_id.clone(), signature_iri.clone()),
        _ => return ExitCode::WrongResponseVerb as i32,
    };

    let output_vec = unsafe { output.as_slice() }.to_vec();
    let dispatched_to_str = unsafe { dispatched_to.as_slice() };
    let dispatched_to_resolved = if dispatched_to_str.is_empty() {
        if fallback_dispatched_to.is_empty() {
            None
        } else {
            Some(fallback_dispatched_to)
        }
    } else {
        Some(
            std::str::from_utf8(dispatched_to_str)
                .map(str::to_owned)
                .unwrap_or(fallback_dispatched_to),
        )
    };

    let response = Response::DispatchOk {
        invocation_id,
        output: serde_bytes::ByteBuf::from(output_vec),
        derivations: Vec::new(),
        dispatched_to: dispatched_to_resolved,
    };
    send_response(handle, response, RequestKindMatch::DispatchMethod)
}

/// Send a [`Response::DispatchFailed`]. `error_kind` and `message`
/// are borrowed UTF-8 byte slices.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_send_dispatch_failed(
    h: *mut WorkerHandle,
    error_kind: Bytes,
    message: Bytes,
) -> i32 {
    let handle = match unsafe { h.as_mut() } {
        Some(h) => h,
        None => return ExitCode::NoInFlightRequest as i32,
    };

    let invocation_id = match &handle.in_flight {
        Some(InFlightRequest::DispatchMethod { invocation_id, .. }) => invocation_id.clone(),
        _ => return ExitCode::WrongResponseVerb as i32,
    };

    let error_kind_str = match std::str::from_utf8(unsafe { error_kind.as_slice() }) {
        Ok(s) => s.to_string(),
        Err(_) => "runtime_error".to_string(),
    };
    let message_str = match std::str::from_utf8(unsafe { message.as_slice() }) {
        Ok(s) => s.to_string(),
        Err(_) => "<non-utf8 diagnostic from worker>".to_string(),
    };

    let response = Response::DispatchFailed {
        invocation_id,
        error_kind: error_kind_str,
        message: message_str,
    };
    send_response(handle, response, RequestKindMatch::DispatchMethod)
}

/// Send a [`Response::Evicted`]. Caller should follow up with
/// [`worker_close`] — the worker doesn't auto-close the stream.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_send_evicted(h: *mut WorkerHandle) -> i32 {
    let handle = match unsafe { h.as_mut() } {
        Some(h) => h,
        None => return ExitCode::NoInFlightRequest as i32,
    };
    let response = Response::Evicted;
    send_response(handle, response, RequestKindMatch::Any)
}

/// Release an [`OwnedBytes`] handed out by the worker. Safe to
/// call with [`OwnedBytes::empty`] (no-op on null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn worker_free_owned_bytes(b: OwnedBytes) {
    if b.ptr.is_null() {
        return;
    }
    let _v = unsafe { Vec::from_raw_parts(b.ptr, b.len, b.cap) };
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Disposition for verb-matching in [`send_response`].
#[derive(Debug, Clone, Copy)]
enum RequestKindMatch {
    /// Response is valid for any in-flight verb (Health,
    /// Instantiated, MirrorRegistered, Evicted).
    Any,
    /// Response must match an in-flight DispatchMethod request.
    DispatchMethod,
}

/// Encode a response, write it to the stream, and clear the
/// in-flight slot.
fn send_response(handle: &mut WorkerHandle, response: Response, expected: RequestKindMatch) -> i32 {
    if !response_matches_in_flight(&handle.in_flight, expected) {
        return ExitCode::WrongResponseVerb as i32;
    }
    let result = match server_send_response(&mut handle.stream, &response) {
        Ok(()) => ExitCode::Ok as i32,
        Err(e) => {
            eprintln!("eigenius-lean-worker: send failed: {e}");
            ExitCode::WriteFailed as i32
        }
    };
    // Clear the in-flight slot regardless of whether the send
    // succeeded — a write failure means the connection is in an
    // indeterminate state and the next operation should be a fresh
    // `worker_next_request_kind` or `worker_close`.
    handle.in_flight = None;
    result
}

fn response_matches_in_flight(
    in_flight: &Option<InFlightRequest>,
    expected: RequestKindMatch,
) -> bool {
    match (in_flight, expected) {
        (None, _) => false,
        (_, RequestKindMatch::Any) => true,
        (Some(InFlightRequest::DispatchMethod { .. }), RequestKindMatch::DispatchMethod) => true,
        (Some(_), RequestKindMatch::DispatchMethod) => false,
    }
}

/// HealthInfo the worker self-reports. Populates
/// `manifest_hash_in_image` / `env_digest_in_image` from env vars
/// the substrate sets (D26 §9.3); missing vars become `None`.
fn default_health_info() -> HealthInfo {
    HealthInfo {
        manifest_hash_in_image: std::env::var("EIGENIUS_RUNTIME_ENV_MANIFEST_HASH").ok(),
        env_digest_in_image: std::env::var("EIGENIUS_RUNTIME_ENV_DIGEST").ok(),
        numerical_metadata: NumericalMetadata::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_runtime_substrate::rpc::client::WorkerRpcClient;
    use eigenius_runtime_substrate::rpc::method::MethodInvocation;
    use std::thread;

    // -----------------------------------------------------------------
    // Helpers
    //
    // The unit tests drive the polling API from a background "Lean-
    // like" thread, exactly the way the Lake-built Lean worker will
    // — modulo the FFI boundary. Each test:
    //
    //   1. Creates a `UnixStream::pair`. The "server" half becomes
    //      the worker's connected stream; the "client" half becomes
    //      the substrate's `WorkerRpcClient`.
    //   2. Spawns the "worker thread" that constructs a
    //      `WorkerHandle` directly (bypassing `worker_listen`'s
    //      bind/accept — tested separately in `tests/uds_round_trip.rs`)
    //      and runs the polling loop until `Evict`.
    //   3. Drives the protocol from the test's main thread via
    //      `WorkerRpcClient`.
    // -----------------------------------------------------------------

    /// Construct a `WorkerHandle` around an already-accepted stream.
    /// Mirrors what `worker_listen` does after `accept`, but skips
    /// the UDS bind for in-process unit tests. Exposed only here.
    fn handle_for_test(stream: UnixStream) -> Box<WorkerHandle> {
        Box::new(WorkerHandle {
            listener: None,
            stream,
            in_flight: None,
        })
    }

    fn spawn_worker_with_loop<F>(handler: F) -> (thread::JoinHandle<()>, WorkerRpcClient)
    where
        F: FnOnce(&mut WorkerHandle) + Send + 'static,
    {
        let (client_side, server_side) = UnixStream::pair().expect("UnixStream::pair");
        let server = thread::spawn(move || {
            let mut handle = handle_for_test(server_side);
            handler(&mut handle);
        });
        let client = WorkerRpcClient::new(client_side);
        (server, client)
    }

    fn encode_method_invocation(function_name: &str) -> serde_bytes::ByteBuf {
        let mi = MethodInvocation {
            function_name: function_name.to_string(),
            signature_iri: format!("urn:eigenius:test:lean:methods:{function_name}"),
        };
        let mut buf = Vec::new();
        ciborium::into_writer(&mi, &mut buf).expect("encode");
        serde_bytes::ByteBuf::from(buf)
    }

    fn owned_to_string(b: OwnedBytes) -> String {
        let s = unsafe { std::slice::from_raw_parts(b.ptr, b.len) }.to_vec();
        let result = String::from_utf8(s).expect("utf-8");
        unsafe { worker_free_owned_bytes(b) };
        result
    }

    fn owned_to_vec(b: OwnedBytes) -> Vec<u8> {
        let v = unsafe { std::slice::from_raw_parts(b.ptr, b.len) }.to_vec();
        unsafe { worker_free_owned_bytes(b) };
        v
    }

    // -----------------------------------------------------------------
    // Polling-API drive: the simplest possible worker loop. Mirrors
    // exactly what the Lake worker's `runLoop` will do.
    // -----------------------------------------------------------------

    /// Run a single iteration of "consume the request, dispatch by
    /// kind, send a response." Returns the [`RequestKind`] tag so the
    /// outer loop can decide whether to exit (`Evict`).
    fn polling_loop_step(handle: &mut WorkerHandle, lean_export_response: &[u8]) -> RequestKind {
        let kind_i32 = unsafe { worker_next_request_kind(handle as *mut _) };
        let kind = decode_kind(kind_i32);
        match kind {
            RequestKind::Health => {
                let _ = unsafe { worker_send_health(handle as *mut _) };
            }
            RequestKind::Instantiate => {
                let _ = unsafe { worker_send_instantiated(handle as *mut _, true) };
            }
            RequestKind::RegisterMirror => {
                let iri = unsafe { worker_request_mirror_iri(handle as *mut _) };
                let iri_str = owned_to_string(iri);
                let _ = unsafe {
                    worker_send_mirror_registered(
                        handle as *mut _,
                        Bytes::from_slice(iri_str.as_bytes()),
                    )
                };
            }
            RequestKind::DispatchMethod => {
                let function_name =
                    owned_to_string(unsafe { worker_request_function_name(handle as *mut _) });
                if function_name == "lean_export" {
                    let _ = unsafe {
                        worker_send_dispatch_ok(
                            handle as *mut _,
                            Bytes::from_slice(lean_export_response),
                            Bytes::from_slice(&[]),
                        )
                    };
                } else {
                    let msg = format!("function `{function_name}` not implemented");
                    let _ = unsafe {
                        worker_send_dispatch_failed(
                            handle as *mut _,
                            Bytes::from_slice(b"not_implemented"),
                            Bytes::from_slice(msg.as_bytes()),
                        )
                    };
                }
            }
            RequestKind::Evict => {
                let _ = unsafe { worker_send_evicted(handle as *mut _) };
            }
            RequestKind::UnsupportedScriptKind => {
                let _ = unsafe {
                    worker_send_dispatch_failed(
                        handle as *mut _,
                        Bytes::from_slice(b"method_signature_mismatch"),
                        Bytes::from_slice(b"Lean worker only handles target_kind = Method"),
                    )
                };
            }
            RequestKind::MalformedMethodInvocation => {
                let _ = unsafe {
                    worker_send_dispatch_failed(
                        handle as *mut _,
                        Bytes::from_slice(b"method_signature_mismatch"),
                        Bytes::from_slice(b"MethodInvocation decode failed"),
                    )
                };
            }
            RequestKind::Closed | RequestKind::TransportError => {
                // No response to send — peer closed or transport
                // broke. The outer loop should exit.
            }
        }
        kind
    }

    fn decode_kind(i: i32) -> RequestKind {
        match i {
            0 => RequestKind::Health,
            1 => RequestKind::Instantiate,
            2 => RequestKind::RegisterMirror,
            3 => RequestKind::DispatchMethod,
            4 => RequestKind::Evict,
            -1 => RequestKind::Closed,
            -2 => RequestKind::TransportError,
            -3 => RequestKind::UnsupportedScriptKind,
            -4 => RequestKind::MalformedMethodInvocation,
            other => panic!("unknown request kind: {other}"),
        }
    }

    // -----------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------

    #[test]
    fn health_request_returns_default_health_info() {
        let (server, mut client) = spawn_worker_with_loop(|h| {
            polling_loop_step(h, b"");
            // Continue serving until Evict.
            loop {
                let k = polling_loop_step(h, b"");
                if matches!(
                    k,
                    RequestKind::Evict | RequestKind::Closed | RequestKind::TransportError
                ) {
                    break;
                }
            }
        });
        let resp = client.call(&Request::Health).expect("health");
        assert!(matches!(resp, Response::Health(_)));
        client.send(&Request::Evict).expect("evict");
        server.join().expect("server join");
    }

    #[test]
    fn instantiate_request_returns_ready_true() {
        let (server, mut client) = spawn_worker_with_loop(|h| loop {
            let k = polling_loop_step(h, b"");
            if matches!(
                k,
                RequestKind::Evict | RequestKind::Closed | RequestKind::TransportError
            ) {
                break;
            }
        });
        let resp = client
            .call(&Request::Instantiate {
                env_iri: "urn:eigenius:test:env".to_string(),
                image_digest: None,
            })
            .expect("instantiate");
        match resp {
            Response::Instantiated { ready } => assert!(ready),
            other => panic!("expected Instantiated, got {other:?}"),
        }
        client.send(&Request::Evict).expect("evict");
        server.join().expect("server join");
    }

    #[test]
    fn dispatch_lean_export_returns_provided_response() {
        let canned_response = b"\x01\x02\x03 lean export bytes";
        let (server, mut client) = spawn_worker_with_loop(move |h| loop {
            let k = polling_loop_step(h, canned_response);
            if matches!(
                k,
                RequestKind::Evict | RequestKind::Closed | RequestKind::TransportError
            ) {
                break;
            }
        });

        let target = encode_method_invocation("lean_export");
        let resp = client
            .call(&Request::DispatchMethod {
                invocation_id: "inv-1".to_string(),
                target_kind: TargetKind::Method,
                target,
                inputs: vec![],
            })
            .expect("dispatch");
        match resp {
            Response::DispatchOk {
                invocation_id,
                output,
                derivations: _,
                dispatched_to,
            } => {
                assert_eq!(invocation_id, "inv-1");
                assert_eq!(output.as_ref(), canned_response);
                assert_eq!(
                    dispatched_to.as_deref(),
                    Some("urn:eigenius:test:lean:methods:lean_export"),
                    "dispatched_to should fall back to the request's signature_iri when the loop passes empty Bytes"
                );
            }
            other => panic!("expected DispatchOk, got {other:?}"),
        }
        client.send(&Request::Evict).expect("evict");
        server.join().expect("server join");
    }

    #[test]
    fn dispatch_unknown_method_returns_dispatch_failed() {
        let (server, mut client) = spawn_worker_with_loop(|h| loop {
            let k = polling_loop_step(h, b"");
            if matches!(
                k,
                RequestKind::Evict | RequestKind::Closed | RequestKind::TransportError
            ) {
                break;
            }
        });
        let target = encode_method_invocation("compute_some_user_thing");
        let resp = client
            .call(&Request::DispatchMethod {
                invocation_id: "inv-2".to_string(),
                target_kind: TargetKind::Method,
                target,
                inputs: vec![],
            })
            .expect("dispatch");
        match resp {
            Response::DispatchFailed {
                invocation_id,
                error_kind,
                message,
            } => {
                assert_eq!(invocation_id, "inv-2");
                assert_eq!(error_kind, "not_implemented");
                assert!(message.contains("compute_some_user_thing"));
            }
            other => panic!("expected DispatchFailed, got {other:?}"),
        }
        client.send(&Request::Evict).expect("evict");
        server.join().expect("server join");
    }

    #[test]
    fn script_target_kind_surfaces_as_unsupported_kind() {
        let (server, mut client) = spawn_worker_with_loop(|h| loop {
            let k = polling_loop_step(h, b"");
            if matches!(
                k,
                RequestKind::Evict | RequestKind::Closed | RequestKind::TransportError
            ) {
                break;
            }
        });
        let resp = client
            .call(&Request::DispatchMethod {
                invocation_id: "inv-3".to_string(),
                target_kind: TargetKind::Script,
                target: serde_bytes::ByteBuf::from(b"some lean source".to_vec()),
                inputs: vec![],
            })
            .expect("dispatch");
        match resp {
            Response::DispatchFailed { error_kind, .. } => {
                assert_eq!(error_kind, "method_signature_mismatch");
            }
            other => panic!("expected DispatchFailed, got {other:?}"),
        }
        client.send(&Request::Evict).expect("evict");
        server.join().expect("server join");
    }

    #[test]
    fn malformed_method_invocation_surfaces_as_decode_error() {
        let (server, mut client) = spawn_worker_with_loop(|h| loop {
            let k = polling_loop_step(h, b"");
            if matches!(
                k,
                RequestKind::Evict | RequestKind::Closed | RequestKind::TransportError
            ) {
                break;
            }
        });
        // Pass garbage bytes as the target — won't decode to a
        // MethodInvocation. The polling layer surfaces this as
        // RequestKind::MalformedMethodInvocation; the test loop
        // sends a DispatchFailed in response.
        let resp = client
            .call(&Request::DispatchMethod {
                invocation_id: "inv-4".to_string(),
                target_kind: TargetKind::Method,
                target: serde_bytes::ByteBuf::from(vec![0xff, 0xff, 0xff]),
                inputs: vec![],
            })
            .expect("dispatch");
        match resp {
            Response::DispatchFailed {
                error_kind,
                message,
                ..
            } => {
                assert_eq!(error_kind, "method_signature_mismatch");
                assert!(message.contains("MethodInvocation decode failed"));
            }
            other => panic!("expected DispatchFailed, got {other:?}"),
        }
        client.send(&Request::Evict).expect("evict");
        server.join().expect("server join");
    }

    #[test]
    fn evict_kind_terminates_loop_after_sending_evicted() {
        let (server, mut client) = spawn_worker_with_loop(|h| loop {
            let k = polling_loop_step(h, b"");
            if matches!(
                k,
                RequestKind::Evict | RequestKind::Closed | RequestKind::TransportError
            ) {
                break;
            }
        });
        let resp = client.call(&Request::Evict).expect("evict");
        assert!(matches!(resp, Response::Evicted));
        server.join().expect("server join");
    }

    #[test]
    fn dispatch_inputs_round_trip_through_accessors() {
        let (server, mut client) = spawn_worker_with_loop(|h| {
            // Single-step loop: read the input bytes back as the
            // DispatchOk output payload. Demonstrates that the field
            // accessors round-trip cleanly for byte-array inputs.
            let _ = unsafe { worker_next_request_kind(h as *mut _) };
            let inputs_count = unsafe { worker_request_input_count(h as *mut _) };
            assert_eq!(inputs_count, 2);
            let first = owned_to_vec(unsafe { worker_request_input(h as *mut _, 0) });
            let second = owned_to_vec(unsafe { worker_request_input(h as *mut _, 1) });
            let combined: Vec<u8> = first.into_iter().chain(second).collect();
            let _ = unsafe {
                worker_send_dispatch_ok(
                    h as *mut _,
                    Bytes::from_slice(&combined),
                    Bytes::from_slice(&[]),
                )
            };
            // Drain Evict.
            loop {
                let k = polling_loop_step(h, b"");
                if matches!(
                    k,
                    RequestKind::Evict | RequestKind::Closed | RequestKind::TransportError
                ) {
                    break;
                }
            }
        });
        let target = encode_method_invocation("lean_export");
        let resp = client
            .call(&Request::DispatchMethod {
                invocation_id: "inv-5".to_string(),
                target_kind: TargetKind::Method,
                target,
                inputs: vec![
                    serde_bytes::ByteBuf::from(vec![0xa1, 0xa2]),
                    serde_bytes::ByteBuf::from(vec![0xb1]),
                ],
            })
            .expect("dispatch");
        match resp {
            Response::DispatchOk { output, .. } => {
                assert_eq!(output.as_ref(), &[0xa1, 0xa2, 0xb1]);
            }
            other => panic!("expected DispatchOk, got {other:?}"),
        }
        client.send(&Request::Evict).expect("evict");
        server.join().expect("server join");
    }

    #[test]
    fn instantiate_accessor_reads_env_iri() {
        // Independent confirmation that the accessors work for
        // verbs other than DispatchMethod.
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let captured_clone = std::sync::Arc::clone(&captured);
        let (server, mut client) = spawn_worker_with_loop(move |h| {
            let _ = unsafe { worker_next_request_kind(h as *mut _) };
            let env_iri = owned_to_string(unsafe { worker_request_env_iri(h as *mut _) });
            *captured_clone.lock().unwrap() = env_iri;
            let _ = unsafe { worker_send_instantiated(h as *mut _, true) };
            loop {
                let k = polling_loop_step(h, b"");
                if matches!(
                    k,
                    RequestKind::Evict | RequestKind::Closed | RequestKind::TransportError
                ) {
                    break;
                }
            }
        });
        client
            .call(&Request::Instantiate {
                env_iri: "urn:eigenius:test:env:lean".to_string(),
                image_digest: None,
            })
            .expect("instantiate");
        client.send(&Request::Evict).expect("evict");
        server.join().expect("server join");
        assert_eq!(
            captured.lock().unwrap().as_str(),
            "urn:eigenius:test:env:lean"
        );
    }

    #[test]
    fn owned_bytes_alloc_zero_is_empty_sentinel() {
        let empty = OwnedBytes::from_vec(Vec::new());
        assert!(empty.is_empty());
        unsafe { worker_free_owned_bytes(empty) };
    }

    #[test]
    fn owned_bytes_alloc_round_trip_through_free() {
        let v: Vec<u8> = (0..64).collect();
        let owned = OwnedBytes::from_vec(v.clone());
        assert_eq!(owned.len, 64);
        let back = unsafe { slice::from_raw_parts(owned.ptr, owned.len) }.to_vec();
        assert_eq!(back, v);
        unsafe { worker_free_owned_bytes(owned) };
    }

    // -----------------------------------------------------------------
    // `worker_decode_eigon_string_property` — Eigon-CBOR decode helper
    //
    // The Lake worker calls this through the FFI to read string
    // properties off `call_method` input Resources (see
    // `runLeanExport`). The decode behaviour is the contract:
    //
    //   - Success: returns the UTF-8 bytes of the matched string
    //     property.
    //   - Any failure (decode error, property absent, value not a
    //     string, IRI malformed): returns empty `OwnedBytes`. The
    //     Lake worker treats empty as "missing input shape" and
    //     dispatches a `DispatchFailed`.
    //
    // These tests exercise the failure paths directly so a future
    // refactor that accidentally changes the empty-on-failure
    // contract gets caught here instead of from a confusing
    // dispatch-level error.
    // -----------------------------------------------------------------

    fn decode_via_internal(cbor: &[u8], iri: &[u8]) -> Option<Vec<u8>> {
        decode_eigon_string_property(cbor, iri)
    }

    fn encoded_resource_with_string(iri: &str, value: &str) -> Vec<u8> {
        use eigenius_kernel::ontology::eigon_cbor;
        use eigenius_kernel::ontology::iri::Iri;
        use eigenius_kernel::ontology::resource::{Resource, Value};
        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse(iri).expect("static IRI"),
            Value::String(value.to_string()),
        );
        eigon_cbor::serialize_resource(&r)
    }

    #[test]
    fn decode_eigon_string_property_round_trips_present_string() {
        // The happy path: caller serialises a Resource carrying a
        // string property, decoder returns the value bytes. This
        // pins the wire contract the Lake worker depends on for
        // every `call_method` dispatch.
        let cbor = encoded_resource_with_string("urn:eigenius:lean:module_name", "TestProject.Foo");
        let bytes = decode_via_internal(&cbor, b"urn:eigenius:lean:module_name")
            .expect("present property must decode");
        assert_eq!(bytes, b"TestProject.Foo");
    }

    #[test]
    fn decode_eigon_string_property_returns_none_when_property_absent() {
        // Resource has `module_name`; caller asks for `constant_name`.
        // No match → None → the FFI surface emits empty OwnedBytes →
        // the Lake worker treats it as the standard "missing input"
        // condition.
        let cbor = encoded_resource_with_string("urn:eigenius:lean:module_name", "TestProject.Foo");
        assert!(decode_via_internal(&cbor, b"urn:eigenius:lean:constant_name").is_none());
    }

    #[test]
    fn decode_eigon_string_property_returns_none_for_non_string_value() {
        // Same property name, but the value is an integer instead of
        // a string. The decoder is intentionally strict: only
        // `Value::String` round-trips through; arrays, integers,
        // resource refs etc. surface as None so the worker doesn't
        // accidentally interpret raw bytes from another shape.
        use eigenius_kernel::ontology::eigon_cbor;
        use eigenius_kernel::ontology::iri::Iri;
        use eigenius_kernel::ontology::resource::{Resource, Value};
        let mut r = Resource::new_embedded();
        r.set(
            Iri::parse("urn:eigenius:lean:module_name").unwrap(),
            Value::Integer(42),
        );
        let cbor = eigon_cbor::serialize_resource(&r);
        assert!(decode_via_internal(&cbor, b"urn:eigenius:lean:module_name").is_none());
    }

    #[test]
    fn decode_eigon_string_property_returns_none_for_malformed_cbor() {
        // Garbage bytes that aren't CBOR at all. `parse_resource_lenient`
        // rejects → None.
        let garbage = b"this is not CBOR";
        assert!(decode_via_internal(garbage, b"urn:eigenius:lean:module_name").is_none());
    }

    #[test]
    fn decode_eigon_string_property_returns_none_for_malformed_iri() {
        // The property IRI bytes aren't a valid IRI. The decoder
        // catches it before touching the resource — bad IRI ≠ missing
        // property semantically, but the empty-bytes surface keeps the
        // failure mode uniform from the worker's perspective.
        let cbor = encoded_resource_with_string("urn:eigenius:lean:module_name", "TestProject.Foo");
        // Spaces aren't allowed in IRIs per RFC 3987 — Iri::parse rejects.
        assert!(decode_via_internal(&cbor, b"not a valid iri").is_none());
    }

    #[test]
    fn decode_eigon_string_property_returns_none_for_non_utf8_iri() {
        // The property IRI must be valid UTF-8 for `Iri::parse` to
        // even see it; raw 0xFF bytes fail the from_utf8 check and
        // bail before touching the CBOR.
        let cbor = encoded_resource_with_string("urn:eigenius:lean:module_name", "TestProject.Foo");
        let invalid_utf8 = [0xFFu8, 0xFE, 0xFD];
        assert!(decode_via_internal(&cbor, &invalid_utf8).is_none());
    }

    #[test]
    fn decode_eigon_string_property_ffi_returns_empty_on_failure() {
        // Cover the FFI wrapper itself — the Option<Vec<u8>>-to-
        // OwnedBytes conversion. A missing property must produce an
        // *empty* OwnedBytes (not null, not garbage); the FFI's
        // empty contract is what `accessor_lean_result` translates
        // into a zero-length Lean ByteArray, which is what the
        // Lake worker tests for via `bytes.size == 0`.
        let cbor = encoded_resource_with_string("urn:eigenius:lean:module_name", "TestProject.Foo");
        let owned = unsafe {
            worker_decode_eigon_string_property(
                Bytes::from_slice(&cbor),
                Bytes::from_slice(b"urn:eigenius:lean:constant_name"),
            )
        };
        assert!(
            owned.is_empty(),
            "missing property must surface as empty OwnedBytes"
        );
        unsafe { worker_free_owned_bytes(owned) };
    }

    #[test]
    fn decode_eigon_string_property_ffi_returns_value_bytes_on_match() {
        // Mirror of the happy path through the FFI surface — pins
        // that the Bytes/OwnedBytes plumbing doesn't drop or
        // corrupt the value bytes between the internal decoder and
        // the FFI return.
        let cbor = encoded_resource_with_string("urn:eigenius:lean:constant_name", "foo");
        let owned = unsafe {
            worker_decode_eigon_string_property(
                Bytes::from_slice(&cbor),
                Bytes::from_slice(b"urn:eigenius:lean:constant_name"),
            )
        };
        let bytes = unsafe { slice::from_raw_parts(owned.ptr, owned.len) }.to_vec();
        assert_eq!(bytes, b"foo");
        unsafe { worker_free_owned_bytes(owned) };
    }
}
