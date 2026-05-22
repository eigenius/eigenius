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

//! Lean-ABI-shaped wrappers around the [`crate`]-level polling API.
//!
//! Each `pub extern "C" fn ei_lean_worker_*` in this file is the
//! symbol a Lean `@[extern]` declaration in
//! [`lean/runtime-worker/Worker/Main.lean`] points at. The wrapper:
//!
//! 1. Decodes Lean argument types (ByteArray via
//!    [`lean_sys::ei_lean_sarray_*`]; external-object handle via
//!    [`lean_sys::ei_lean_get_external_data`]; scalars via
//!    [`lean_sys::ei_lean_unbox`]).
//! 2. Calls the matching `worker_*` polling-API entry point.
//! 3. Encodes the result back into a Lean object (ByteArray copy,
//!    boxed scalar, external-object allocation) and wraps it in an
//!    `IO.Result.ok` via [`lean_sys::ei_lean_io_result_mk_ok`].
//!
//! The IO world token (the final `lean_obj_arg` Lean's ABI passes
//! after every `IO α`-returning function's user-visible args) is
//! ignored — IO is a "world-passing" formalism in Lean; for FFI
//! you accept the token and don't use it.
//!
//! ## External-class registration
//!
//! Lean represents our [`WorkerHandle`](crate::WorkerHandle) as an
//! external Lean object (`lean_external_object`) so the GC can
//! finalise it deterministically. The class itself
//! (finalize + foreach callbacks) is registered lazily, once per
//! process, via [`worker_handle_class`].

use std::ffi::c_void;
use std::sync::OnceLock;

use crate::{
    worker_close, worker_decode_eigon_string_property, worker_free_owned_bytes, worker_listen,
    worker_next_request_kind, worker_request_env_iri, worker_request_function_name,
    worker_request_image_digest, worker_request_input, worker_request_input_count,
    worker_request_invocation_id, worker_request_library_content, worker_request_mirror_iri,
    worker_request_signature_iri, worker_send_dispatch_failed, worker_send_dispatch_ok,
    worker_send_evicted, worker_send_health, worker_send_instantiated,
    worker_send_mirror_registered, Bytes, OwnedBytes, WorkerHandle,
};

use crate::lean_sys::{
    ei_lean_alloc_byte_array, ei_lean_alloc_external, ei_lean_box, ei_lean_box_usize,
    ei_lean_get_external_data, ei_lean_io_result_mk_ok, ei_lean_register_external_class,
    ei_lean_sarray_cptr, ei_lean_sarray_size, LeanExternalClass, LeanObj,
};

// ---------------------------------------------------------------------------
// External-class registration for `WorkerHandle`
// ---------------------------------------------------------------------------

/// Process-global registration record for the WorkerHandle Lean
/// external class. Initialised lazily on the first
/// [`ei_lean_worker_listen`] call — Lean's runtime expects the
/// returned class pointer to be the SAME pointer for every
/// external-object instance of that class, so we cache it.
static WORKER_HANDLE_CLASS: OnceLock<usize> = OnceLock::new();

/// Look up (initialising on first call) the
/// [`lean_external_class`] handle for `WorkerHandle`.
///
/// `OnceLock<usize>` instead of `OnceLock<*mut LeanExternalClass>`
/// because raw pointers aren't `Send`/`Sync` by default but the
/// underlying pointer Lean returns is process-global and
/// thread-safe (the class registration outlives the Lean runtime
/// and is never mutated post-registration).
fn worker_handle_class() -> *mut LeanExternalClass {
    let cached = WORKER_HANDLE_CLASS.get_or_init(|| {
        let cls = unsafe {
            ei_lean_register_external_class(worker_handle_finalize, worker_handle_foreach)
        };
        cls as usize
    });
    *cached as *mut LeanExternalClass
}

/// Finalize callback Lean's GC invokes when a wrapping external
/// object is collected. The `data` pointer was given to
/// [`ei_lean_alloc_external`] when we wrapped a fresh
/// [`WorkerHandle`]; we cast back and delegate to
/// [`worker_close`].
///
/// Safe even if `data` is null because `worker_close` no-ops on
/// null.
unsafe extern "C" fn worker_handle_finalize(data: *mut c_void) {
    let handle = data as *mut WorkerHandle;
    unsafe { worker_close(handle) };
}

/// Foreach callback Lean's GC invokes to enumerate live references
/// from this external object's payload. `WorkerHandle` holds no
/// Lean object references — only Rust-owned state — so this is a
/// no-op.
unsafe extern "C" fn worker_handle_foreach(_data: *mut c_void, _callback: *mut LeanObj) {}

// ---------------------------------------------------------------------------
// Internal helpers — convert between Lean ByteArrays and Rust byte slices.
// ---------------------------------------------------------------------------

/// View a Lean ByteArray as a borrowed `&[u8]`. The slice's
/// lifetime is bounded by Lean's ownership of the ByteArray.
///
/// # Safety
/// `arr` must be a valid Lean ByteArray pointer or null. Caller
/// must not retain the slice past the call boundary (Lean's GC
/// may reclaim).
unsafe fn lean_byte_array_view<'a>(arr: *mut LeanObj) -> &'a [u8] {
    if arr.is_null() {
        return &[];
    }
    let len = unsafe { ei_lean_sarray_size(arr) };
    if len == 0 {
        return &[];
    }
    let ptr = unsafe { ei_lean_sarray_cptr(arr) };
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// Copy `bytes` into a freshly-allocated Lean ByteArray. Lean's
/// runtime owns the returned object after this point.
unsafe fn lean_byte_array_from_slice(bytes: &[u8]) -> *mut LeanObj {
    let arr = unsafe { ei_lean_alloc_byte_array(bytes.len(), bytes.len()) };
    if !bytes.is_empty() {
        let dest = unsafe { ei_lean_sarray_cptr(arr) };
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, bytes.len()) };
    }
    arr
}

/// Wrap an `OwnedBytes` (from a polling-API accessor) as a Lean
/// ByteArray + `IO.Result.ok`. Always frees the OwnedBytes.
unsafe fn accessor_lean_result(owned: OwnedBytes) -> *mut LeanObj {
    let arr = if owned.is_empty() {
        unsafe { ei_lean_alloc_byte_array(0, 0) }
    } else {
        let view = unsafe { std::slice::from_raw_parts(owned.ptr, owned.len) };
        unsafe { lean_byte_array_from_slice(view) }
    };
    unsafe { worker_free_owned_bytes(owned) };
    unsafe { ei_lean_io_result_mk_ok(arr) }
}

/// Wrap `IO Unit` — a freshly-boxed Lean `Unit` value (`lean_box(0)`)
/// inside `IO.Result.ok`.
unsafe fn unit_lean_result() -> *mut LeanObj {
    let unit = unsafe { ei_lean_box(0) };
    unsafe { ei_lean_io_result_mk_ok(unit) }
}

/// Wrap an `Int32` / `UInt32` scalar in `IO α`. On 64-bit
/// platforms these fit in a tagged pointer; `lean_box` does the
/// right thing.
unsafe fn scalar_uint32_lean_result(value: usize) -> *mut LeanObj {
    let scalar = unsafe { ei_lean_box(value) };
    unsafe { ei_lean_io_result_mk_ok(scalar) }
}

/// Wrap a `USize` scalar in `IO USize`. USize is 64-bit on 64-bit
/// platforms and doesn't fit a tagged pointer — Lean's
/// `lean_unbox_usize` reads from a heap-allocated ctor's scalar
/// payload, not from a tagged pointer. So we must use
/// `lean_box_usize` (a separate inline that allocates the ctor)
/// rather than the small-integer `lean_box`.
unsafe fn scalar_usize_lean_result(value: usize) -> *mut LeanObj {
    let scalar = unsafe { ei_lean_box_usize(value) };
    unsafe { ei_lean_io_result_mk_ok(scalar) }
}

/// Extract the raw [`WorkerHandle`] pointer from a Lean external
/// object wrapper. Caller asserts the object is one this crate
/// allocated (Lean's FFI ABI doesn't class-check).
unsafe fn handle_from_lean(obj: *mut LeanObj) -> *mut WorkerHandle {
    if obj.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { ei_lean_get_external_data(obj) as *mut WorkerHandle }
}

// ---------------------------------------------------------------------------
// `worker_listen` / `worker_close` analogues
// ---------------------------------------------------------------------------

/// Lean signature:
/// `@[extern "ei_lean_worker_listen"] opaque listen (path : @& ByteArray) : IO WorkerHandle`
///
/// Path is a UTF-8 ByteArray (not null-terminated). On success
/// returns an external-object Lean wrapping the
/// [`WorkerHandle`]; Lean's GC will eventually finalise it via
/// [`worker_handle_finalize`] — which calls
/// [`worker_close`] — releasing the underlying UDS connection.
///
/// On failure (`worker_listen` returned null), we still wrap a
/// null pointer in the external object and let Lean drive
/// `worker_next_request_kind` on it, which will surface
/// [`crate::RequestKind::TransportError`]. The downside (null
/// handle that "looks valid" from Lean) is acceptable because
/// `worker_next_request_kind` is null-safe.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_listen(path: *mut LeanObj) -> *mut LeanObj {
    let path_slice = unsafe { lean_byte_array_view(path) };
    let handle = unsafe { worker_listen(path_slice.as_ptr(), path_slice.len()) };
    let external = unsafe { ei_lean_alloc_external(worker_handle_class(), handle as *mut c_void) };
    unsafe { ei_lean_io_result_mk_ok(external) }
}

// ---------------------------------------------------------------------------
// Request kind + field accessors
// ---------------------------------------------------------------------------

/// Lean signature:
/// `@[extern "ei_lean_worker_accept_next"] opaque acceptNext (h : @& WorkerHandle) : IO Int32`
///
/// Close the current substrate connection and accept the next one
/// on the bound listener. Returns 0 on success, non-zero on
/// accept failure. See [`crate::worker_accept_next`] for the
/// per-RPC connection-loop rationale.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_accept_next(handle_obj: *mut LeanObj) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let rc = unsafe { crate::worker_accept_next(handle) };
    // Same bit-preserving widening as `next_request_kind` — Lean's
    // Int32 round-trips through `i32 as u32 as usize`.
    unsafe { scalar_uint32_lean_result(rc as u32 as usize) }
}

/// Lean signature:
/// `@[extern "ei_lean_worker_next_request_kind"] opaque nextRequestKind (h : @& WorkerHandle) : IO Int32`
///
/// Blocks until the next request frame arrives, decodes it into
/// the handle's in-flight slot, returns the discriminator as a
/// boxed scalar. Lean side casts/decodes to its `RequestKind`
/// inductive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_next_request_kind(
    handle_obj: *mut LeanObj,
) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let kind = unsafe { worker_next_request_kind(handle) };
    // Lean's `Int32` is signed; we widen via cast through `usize`
    // safely because `Int32` boxed representation is just the
    // unboxed value with its low bits used as tag. Negative kinds
    // (the error variants) round-trip because `i32 as usize` on
    // two's-complement preserves the bit pattern, and Lean reads
    // it back as an Int32 via the same convention.
    unsafe { scalar_uint32_lean_result(kind as u32 as usize) }
}

/// Generate accessor wrappers. Each delegates to the matching
/// polling-API function and wraps the result via
/// [`accessor_lean_result`].
macro_rules! lean_ffi_accessor {
    ($lean_name:ident, $rust_name:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $lean_name(handle_obj: *mut LeanObj) -> *mut LeanObj {
            let handle = unsafe { handle_from_lean(handle_obj) };
            let owned = unsafe { $rust_name(handle) };
            unsafe { accessor_lean_result(owned) }
        }
    };
}

lean_ffi_accessor!(ei_lean_worker_request_env_iri, worker_request_env_iri);
lean_ffi_accessor!(
    ei_lean_worker_request_image_digest,
    worker_request_image_digest
);
lean_ffi_accessor!(ei_lean_worker_request_mirror_iri, worker_request_mirror_iri);
lean_ffi_accessor!(
    ei_lean_worker_request_library_content,
    worker_request_library_content
);
lean_ffi_accessor!(
    ei_lean_worker_request_invocation_id,
    worker_request_invocation_id
);
lean_ffi_accessor!(
    ei_lean_worker_request_function_name,
    worker_request_function_name
);
lean_ffi_accessor!(
    ei_lean_worker_request_signature_iri,
    worker_request_signature_iri
);

/// Lean signature:
/// `@[extern "ei_lean_worker_request_input_count"] opaque requestInputCount (h : @& WorkerHandle) : IO USize`
///
/// Lean's compiled C ABI for `IO α`-returning externs *does not*
/// pass the `IO.RealWorld` token through — it's compiled away
/// before the FFI call. Hence the single-argument signature (the
/// borrowed handle); the wrapper code in
/// `.lake/build/ir/Worker/Ffi.c` matches this shape.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_request_input_count(
    handle_obj: *mut LeanObj,
) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let count = unsafe { worker_request_input_count(handle) };
    unsafe { scalar_usize_lean_result(count) }
}

/// Lean signature:
/// `@[extern "ei_lean_worker_request_input"] opaque requestInput (h : @& WorkerHandle) (index : USize) : IO ByteArray`
///
/// Lean's `USize` is unboxed by the compiler when passed as an
/// argument to an `@[extern]` function — the C wrapper sees a
/// plain `size_t`, not a Lean object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_request_input(
    handle_obj: *mut LeanObj,
    index: usize,
) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let owned = unsafe { worker_request_input(handle, index) };
    unsafe { accessor_lean_result(owned) }
}

// ---------------------------------------------------------------------------
// Eigon-CBOR decoders
// ---------------------------------------------------------------------------

/// Lean signature:
/// `@[extern "ei_lean_worker_decode_eigon_string_property"]`
/// `opaque decodeEigonStringProperty (cbor : @& ByteArray) (propertyIri : @& ByteArray) : IO ByteArray`
///
/// Pure decode helper — no [`WorkerHandle`] involved. Parses the
/// `cbor` argument as an Eigon Resource and returns the UTF-8 bytes
/// of `propertyIri` if present as a string property. Empty bytes
/// on any failure (decode error, property absent, value not a
/// string); the caller is expected to dispatch a `DispatchFailed`
/// in that case.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_decode_eigon_string_property(
    cbor_obj: *mut LeanObj,
    property_iri_obj: *mut LeanObj,
) -> *mut LeanObj {
    let cbor_slice = unsafe { lean_byte_array_view(cbor_obj) };
    let iri_slice = unsafe { lean_byte_array_view(property_iri_obj) };
    let owned = unsafe {
        worker_decode_eigon_string_property(
            Bytes {
                ptr: cbor_slice.as_ptr(),
                len: cbor_slice.len(),
            },
            Bytes {
                ptr: iri_slice.as_ptr(),
                len: iri_slice.len(),
            },
        )
    };
    unsafe { accessor_lean_result(owned) }
}

// ---------------------------------------------------------------------------
// Senders
// ---------------------------------------------------------------------------

/// Lean signature:
/// `@[extern "ei_lean_worker_send_health"] opaque sendHealth (h : @& WorkerHandle) : IO Unit`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_send_health(handle_obj: *mut LeanObj) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let _ = unsafe { worker_send_health(handle) };
    unsafe { unit_lean_result() }
}

/// Lean signature:
/// `@[extern "ei_lean_worker_send_instantiated"] opaque sendInstantiated (h : @& WorkerHandle) (ready : Bool) : IO Unit`
///
/// `Bool` is unboxed across the FFI — C ABI gets a `uint8_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_send_instantiated(
    handle_obj: *mut LeanObj,
    ready: u8,
) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let _ = unsafe { worker_send_instantiated(handle, ready != 0) };
    unsafe { unit_lean_result() }
}

/// Lean signature:
/// `@[extern "ei_lean_worker_send_mirror_registered"] opaque sendMirrorRegistered (h : @& WorkerHandle) (mirrorIri : @& ByteArray) : IO Unit`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_send_mirror_registered(
    handle_obj: *mut LeanObj,
    mirror_iri: *mut LeanObj,
) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let iri_slice = unsafe { lean_byte_array_view(mirror_iri) };
    let _ = unsafe { worker_send_mirror_registered(handle, Bytes::from_slice(iri_slice)) };
    unsafe { unit_lean_result() }
}

/// Lean signature:
/// `@[extern "ei_lean_worker_send_dispatch_ok"] opaque sendDispatchOk (h : @& WorkerHandle) (output : @& ByteArray) (dispatchedTo : @& ByteArray) : IO Unit`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_send_dispatch_ok(
    handle_obj: *mut LeanObj,
    output: *mut LeanObj,
    dispatched_to: *mut LeanObj,
) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let output_slice = unsafe { lean_byte_array_view(output) };
    let dispatched_slice = unsafe { lean_byte_array_view(dispatched_to) };
    let _ = unsafe {
        worker_send_dispatch_ok(
            handle,
            Bytes::from_slice(output_slice),
            Bytes::from_slice(dispatched_slice),
        )
    };
    unsafe { unit_lean_result() }
}

/// Lean signature:
/// `@[extern "ei_lean_worker_send_dispatch_failed"] opaque sendDispatchFailed (h : @& WorkerHandle) (errorKind : @& ByteArray) (message : @& ByteArray) : IO Unit`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_send_dispatch_failed(
    handle_obj: *mut LeanObj,
    error_kind: *mut LeanObj,
    message: *mut LeanObj,
) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let error_kind_slice = unsafe { lean_byte_array_view(error_kind) };
    let message_slice = unsafe { lean_byte_array_view(message) };
    let _ = unsafe {
        worker_send_dispatch_failed(
            handle,
            Bytes::from_slice(error_kind_slice),
            Bytes::from_slice(message_slice),
        )
    };
    unsafe { unit_lean_result() }
}

/// Lean signature:
/// `@[extern "ei_lean_worker_send_evicted"] opaque sendEvicted (h : @& WorkerHandle) : IO Unit`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_send_evicted(handle_obj: *mut LeanObj) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let _ = unsafe { worker_send_evicted(handle) };
    unsafe { unit_lean_result() }
}

// ---------------------------------------------------------------------------
// LeanProject staging — Lean-side entry into the worker's
// `lean_project` module.
// ---------------------------------------------------------------------------

/// Lean signature:
/// `@[extern "ei_lean_worker_stage_lean_project"] opaque stageLeanProject (h : @& WorkerHandle) (inputIdx : USize) (destDir : @& ByteArray) : IO ByteArray`
///
/// Reads `input[inputIdx]` (a `LeanProject` Eigon-CBOR resource),
/// decodes it via [`crate::lean_project::stage_lean_project`], and
/// materialises the project's files under `destDir` (UTF-8 path).
///
/// **Return value semantics**: empty `ByteArray` = success;
/// non-empty = UTF-8 error message. The Lean side checks
/// `result.size == 0` for the success case and uses the bytes as
/// the diagnostic for `DispatchFailed` otherwise.
///
/// Empty-success is a deliberate convention — it lets the FFI
/// shape stay `IO ByteArray` (a single Lean-friendly POD return)
/// rather than `IO (Except String Unit)` which would require Lean
/// runtime types we'd have to construct from Rust.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ei_lean_worker_stage_lean_project(
    handle_obj: *mut LeanObj,
    input_idx: usize,
    dest_dir: *mut LeanObj,
) -> *mut LeanObj {
    let handle = unsafe { handle_from_lean(handle_obj) };
    let input_owned = unsafe { crate::worker_request_input(handle, input_idx) };

    let dest_dir_slice = unsafe { lean_byte_array_view(dest_dir) };
    let dest_dir_str = match std::str::from_utf8(dest_dir_slice) {
        Ok(s) => s,
        Err(_) => {
            unsafe { crate::worker_free_owned_bytes(input_owned) };
            return unsafe {
                accessor_lean_result(crate::OwnedBytes::from_string(
                    "destDir is not valid UTF-8".to_string(),
                ))
            };
        }
    };

    let input_view = if input_owned.is_empty() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(input_owned.ptr, input_owned.len) }
    };

    let result =
        crate::lean_project::stage_lean_project(input_view, std::path::Path::new(dest_dir_str));

    unsafe { crate::worker_free_owned_bytes(input_owned) };

    let response = match result {
        Ok(()) => crate::OwnedBytes::empty(),
        Err(e) => crate::OwnedBytes::from_string(e.to_string()),
    };
    unsafe { accessor_lean_result(response) }
}
