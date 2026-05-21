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

//! Hand-rolled Rust bindings to the Lean runtime symbols our FFI
//! bridge calls — both the C thunks declared in [`c/lean_bridge.h`]
//! (re-exposing lean.h's `static inline` helpers) and the Lean
//! runtime's already-linkable `LEAN_EXPORT` symbols we use directly
//! (none today; reserved for future verbs that need richer Lean
//! types like `String` / `Float`).
//!
//! Kept ~50 LOC instead of binding the full lean.h via `bindgen`
//! because we use about a dozen runtime functions and the
//! audit-by-eye surface is small enough to be worth more than the
//! generator's churn-resistance.

use core::ffi::c_void;

/// Opaque Lean object pointer. Matches `EiLeanObj` from the C
/// bridge header — Rust never dereferences this, only passes it
/// through the C thunks.
#[repr(C)]
pub struct LeanObj {
    _private: [u8; 0],
}

/// Opaque external-class handle. Matches `EiLeanExternalClass`.
/// Pointer to a Lean runtime registry record that pairs a finalize
/// and foreach callback with a class identity Lean's GC uses for
/// type-checking external-object payloads.
#[repr(C)]
pub struct LeanExternalClass {
    _private: [u8; 0],
}

/// Lean's external-finalize callback signature. Invoked by Lean's
/// GC when the wrapping object is collected; the `void*` arg is
/// the same `data` pointer that was passed to
/// [`ei_lean_alloc_external`].
pub type LeanFinalizeProc = unsafe extern "C" fn(*mut c_void);

/// Lean's external-foreach callback signature. Visits any Lean
/// objects the external data references (so the GC can trace
/// liveness). Our worker handle holds no Lean references, so we
/// install a no-op foreach.
pub type LeanForeachProc = unsafe extern "C" fn(*mut c_void, *mut LeanObj);

unsafe extern "C" {

    // ----- ByteArray (sarray<u8>) operations ---------------------

    pub fn ei_lean_alloc_byte_array(size: usize, capacity: usize) -> *mut LeanObj;

    pub fn ei_lean_sarray_size(arr: *mut LeanObj) -> usize;

    pub fn ei_lean_sarray_cptr(arr: *mut LeanObj) -> *mut u8;

    // ----- IO results ---------------------------------------------

    pub fn ei_lean_io_result_mk_ok(val: *mut LeanObj) -> *mut LeanObj;

    pub fn ei_lean_io_result_mk_error(err: *mut LeanObj) -> *mut LeanObj;

    pub fn ei_lean_io_mk_world() -> *mut LeanObj;

    // ----- Scalars (box / unbox) ----------------------------------

    pub fn ei_lean_box(n: usize) -> *mut LeanObj;

    pub fn ei_lean_unbox(o: *mut LeanObj) -> usize;

    pub fn ei_lean_box_usize(v: usize) -> *mut LeanObj;

    // ----- External objects ---------------------------------------

    pub fn ei_lean_register_external_class(
        finalize: LeanFinalizeProc,
        foreach: LeanForeachProc,
    ) -> *mut LeanExternalClass;

    pub fn ei_lean_alloc_external(cls: *mut LeanExternalClass, data: *mut c_void) -> *mut LeanObj;

    pub fn ei_lean_get_external_data(o: *mut LeanObj) -> *mut c_void;
}
