/-
Copyright 2026 The Eigenius Authors

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-/

/-!
# Lean-side FFI declarations for the `eigenius-lean-worker` cdylib

Each `@[extern]` binding here points at a symbol exposed by
[`crates/eigenius-lean-worker/src/lean_ffi.rs`](../../crates/eigenius-lean-worker/src/lean_ffi.rs).
The Rust side handles UDS + CBOR transport; the polling loop in
[`Worker.Main`](Main.lean) drives this surface.

## Type-mapping conventions

| Lean type | C ABI shape | Rust side |
|---|---|---|
| `@& ByteArray` | `b_lean_obj_arg` (borrowed object ptr) | `*mut LeanObj` accessed via `ei_lean_sarray_cptr` / `ei_lean_sarray_size` |
| `ByteArray` (return) | `lean_obj_res` (owned object ptr) | freshly allocated via `ei_lean_alloc_byte_array` + memcpy |
| `@& WorkerHandle` | `b_lean_obj_arg` (borrowed external object ptr) | `*mut LeanObj` accessed via `ei_lean_get_external_data` |
| `WorkerHandle` (return) | `lean_obj_res` | wrapped via `ei_lean_alloc_external` |
| `USize` arg / return | `size_t` direct | `usize` direct (passed/returned unboxed) |
| `Int32` return | boxed scalar in `IO.Result.ok` | `i32 as u32 as usize` then `ei_lean_box` |
| `Bool` arg | `uint8_t` direct | `u8` direct |
| `IO Unit` | `IO.Result.ok (lean_box 0)` | `ei_lean_box(0)` wrapped in `ei_lean_io_result_mk_ok` |

The final implicit `lean_obj_arg` (the `IO.RealWorld` token) sits
at the end of every `IO α`-returning function's C ABI signature.
The Rust side accepts and ignores it.
-/

namespace Worker.Ffi

/-! ## `WorkerHandle` — opaque external-object type

Lean's GC tracks lifetime via the `lean_external_object` wrapper
the Rust side allocates in `ei_lean_worker_listen`. When the last
Lean reference is dropped, the registered finalize callback fires
`worker_close`, releasing the UDS connection.

The `Nonempty` instance is required by Lean's elaborator — opaque
types must be inhabited to participate in pattern matching and
`Option`-style algebra. The `WorkerHandle` we materialise in
`listen` is genuinely a non-null pointer (or the IO action fails),
so the inhabitedness claim holds in practice; the
`NonemptyType.{0}` axiom lets us declare it without exposing the
Rust-side payload structure to Lean's type checker.
-/

opaque WorkerHandlePointed : NonemptyType.{0}

/-- Opaque Rust-side worker state. Lean callers obtain one via
[`listen`] and pass it through every subsequent `@& WorkerHandle`
argument. -/
def WorkerHandle : Type := WorkerHandlePointed.type

instance : Nonempty WorkerHandle := WorkerHandlePointed.property

/-! ## Listen / next request -/

/-- Bind a UDS at `path` (UTF-8 byte slice, *not* null-terminated)
and accept one substrate connection. Returns the opaque handle the
rest of this module operates on.

If the bind or accept fails, the underlying Rust call returns null
and the handle is "valid" from Lean's perspective but null inside;
the first [`nextRequestKind`] call will then surface a transport
error (a negative kind). The handle should still be released to
clean up Lean's external-object wrapper. -/
@[extern "ei_lean_worker_listen"]
opaque listen (path : @& ByteArray) : IO WorkerHandle

/-- Close the current substrate connection and accept the next one
on the bound listener — same handle, same UDS. The substrate
opens a fresh UDS connection per RPC (Health and DispatchMethod
are separate dials in D26 §8.1 Service mode), so the worker has
to loop back to accept after each peer close.

Returns `0` on success, non-zero on accept failure. A non-zero
return generally means the listener is dead and the worker
should exit. -/
@[extern "ei_lean_worker_accept_next"]
opaque acceptNext (h : @& WorkerHandle) : IO Int32

/-- Block until the next substrate request arrives, decode it
into the worker's in-flight slot, and return the verb
discriminator as an `Int32`.

Positive values name a request verb the field-accessor helpers
can read out of:
 - `0` Health, `1` Instantiate, `2` RegisterMirror,
 - `3` DispatchMethod, `4` Evict.

Negative values surface protocol-level conditions:
 - `-1` Closed (peer closed cleanly; exit the loop)
 - `-2` TransportError (CBOR decode or I/O failure)
 - `-3` UnsupportedScriptKind (`target_kind = Script`)
 - `-4` MalformedMethodInvocation (`MethodInvocation` decode failed)

For `-3` / `-4`, the caller's expected response is
[`sendDispatchFailed`]; the in-flight slot still carries the
invocation id needed to build the response. -/
@[extern "ei_lean_worker_next_request_kind"]
opaque nextRequestKind (h : @& WorkerHandle) : IO Int32

/-! ## Field accessors

Read fields out of the worker's in-flight request. Each accessor
returns a freshly-allocated `ByteArray` (or `USize` for the input
count). Calling an accessor when no request is in flight, or when
the field doesn't apply to the current verb, returns an empty
`ByteArray` — the polling loop is expected to inspect
[`nextRequestKind`] first and only call the relevant accessors. -/

/-- Instantiate's `env_iri`. -/
@[extern "ei_lean_worker_request_env_iri"]
opaque requestEnvIri (h : @& WorkerHandle) : IO ByteArray

/-- Instantiate's `image_digest`. Empty when the substrate didn't
supply one (LocalSpawner mode). -/
@[extern "ei_lean_worker_request_image_digest"]
opaque requestImageDigest (h : @& WorkerHandle) : IO ByteArray

/-- RegisterMirror's `mirror_iri`. -/
@[extern "ei_lean_worker_request_mirror_iri"]
opaque requestMirrorIri (h : @& WorkerHandle) : IO ByteArray

/-- RegisterMirror's `library_content` archive bytes. -/
@[extern "ei_lean_worker_request_library_content"]
opaque requestLibraryContent (h : @& WorkerHandle) : IO ByteArray

/-- DispatchMethod's `invocation_id`. -/
@[extern "ei_lean_worker_request_invocation_id"]
opaque requestInvocationId (h : @& WorkerHandle) : IO ByteArray

/-- DispatchMethod's `function_name` (pre-decoded from the
`MethodInvocation` payload by the Rust side; Lean dispatches on
this string). -/
@[extern "ei_lean_worker_request_function_name"]
opaque requestFunctionName (h : @& WorkerHandle) : IO ByteArray

/-- DispatchMethod's `signature_iri` (the
`RuntimeMethodSignature` IRI to echo on `DispatchOk.dispatched_to`). -/
@[extern "ei_lean_worker_request_signature_iri"]
opaque requestSignatureIri (h : @& WorkerHandle) : IO ByteArray

/-- DispatchMethod's positional-input count. -/
@[extern "ei_lean_worker_request_input_count"]
opaque requestInputCount (h : @& WorkerHandle) : IO USize

/-- DispatchMethod's positional input at the given index. Empty if
out of range. -/
@[extern "ei_lean_worker_request_input"]
opaque requestInput (h : @& WorkerHandle) (idx : USize) : IO ByteArray

/-! ## Eigon-CBOR decoders

The substrate ships every `call_method` input as Eigon-CBOR. The
cdylib hosts the workspace's Eigon-CBOR codec; these externs let
the worker pull individual property values out of those bytes
without re-implementing CBOR on the Lean side. -/

/-- Parse `cbor` as an Eigon Resource and return the UTF-8 bytes of
its `propertyIri` string property. Empty `ByteArray` on any
failure: the bytes don't decode as a Resource, `propertyIri` is
malformed, the property is absent, or the value isn't a string
(numbers / arrays / nested resources all surface as empty).

The caller should treat an empty return as "input doesn't carry
the expected property" and dispatch a `DispatchFailed` with a
descriptive error_kind. -/
@[extern "ei_lean_worker_decode_eigon_string_property"]
opaque decodeEigonStringProperty (cbor : @& ByteArray)
    (propertyIri : @& ByteArray) : IO ByteArray

/-! ## Senders

Build and send the response for the current in-flight request.
Each sender clears the in-flight slot after sending; the next
[`nextRequestKind`] call sets up a fresh slot. -/

/-- Send `Response::Health` with default worker-self-reported
info. -/
@[extern "ei_lean_worker_send_health"]
opaque sendHealth (h : @& WorkerHandle) : IO Unit

/-- Send `Response::Instantiated{ready}`. -/
@[extern "ei_lean_worker_send_instantiated"]
opaque sendInstantiated (h : @& WorkerHandle) (ready : Bool) : IO Unit

/-- Send `Response::MirrorRegistered{mirror_iri}`. -/
@[extern "ei_lean_worker_send_mirror_registered"]
opaque sendMirrorRegistered (h : @& WorkerHandle) (mirrorIri : @& ByteArray) : IO Unit

/-- Send `Response::DispatchOk{output, dispatched_to}`. Pass an
empty `ByteArray` for `dispatchedTo` to fall back to the
in-flight request's `signature_iri`. -/
@[extern "ei_lean_worker_send_dispatch_ok"]
opaque sendDispatchOk (h : @& WorkerHandle) (output : @& ByteArray)
    (dispatchedTo : @& ByteArray) : IO Unit

/-- Send `Response::DispatchFailed{error_kind, message}`. -/
@[extern "ei_lean_worker_send_dispatch_failed"]
opaque sendDispatchFailed (h : @& WorkerHandle) (errorKind : @& ByteArray)
    (message : @& ByteArray) : IO Unit

/-- Send `Response::Evicted`. -/
@[extern "ei_lean_worker_send_evicted"]
opaque sendEvicted (h : @& WorkerHandle) : IO Unit

/-! ## LeanProject staging

Decode `input[inputIdx]` as a `LeanProject` Eigon-CBOR resource and
materialise its files (`lakefile.toml` / `lakefile.lean`,
`lake-manifest.json`, `lean-toolchain`, and each `source_tree`
entry) under `destDir`.

Returns an empty `ByteArray` on success, or a UTF-8 error message
on failure — the polling loop checks `result.size == 0` and emits
`DispatchFailed` carrying the bytes if non-empty. -/
@[extern "ei_lean_worker_stage_lean_project"]
opaque stageLeanProject (h : @& WorkerHandle) (inputIdx : USize)
    (destDir : @& ByteArray) : IO ByteArray

end Worker.Ffi
