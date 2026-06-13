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

//! R `.Call` bridge over [`RWorker`] (D55, P1.2).
//!
//! The `r_*` functions are `#[no_mangle] extern "C" fn(SEXP…) -> SEXP` —
//! the entry points the R driver calls via `.Call("r_listen", …)` after
//! `dyn.load`. Writing them in Rust (rather than a `cc`-compiled C bridge)
//! guarantees they are exported from the cdylib; a C bridge's symbols are
//! GC'd as unreferenced by Rust and never reach the dynamic symbol table.
//!
//! Only a small slice of libR's stable C API is needed (declared in the
//! [`rapi`] extern block). Those symbols are undefined in the cdylib and
//! resolve at `dyn.load` against the already-loaded libR — exactly how an
//! R package's shared object works. No bindgen, no libR link, no C.
//!
//! The worker lives entirely in Rust; R holds only an `i32` handle id into
//! a process-global registry, so no raw Rust pointers cross the boundary.

use std::collections::HashMap;
use std::os::raw::{c_char, c_int};
use std::slice;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use eigenius_runtime_substrate::rpc::protocol::HealthInfo;

use crate::RWorker;

/// The slice of libR's C API this bridge calls. `SEXP` is an opaque
/// pointer; the functions are the un-remapped (`Rf_`-prefixed) stable
/// entry points plus the data accessors. Resolved at `dyn.load`.
mod rapi {
    use std::os::raw::{c_char, c_int};

    // `SEXP` / `R_xlen_t` keep R's canonical C names verbatim.
    #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
    pub type SEXP = *mut std::os::raw::c_void;
    #[allow(non_camel_case_types)]
    pub type R_xlen_t = isize;

    /// `RAWSXP` — the raw-vector `SEXPTYPE`.
    pub const RAWSXP: c_int = 24;

    unsafe extern "C" {
        pub fn Rf_allocVector(stype: c_int, n: R_xlen_t) -> SEXP;
        pub fn Rf_ScalarInteger(x: c_int) -> SEXP;
        pub fn Rf_ScalarString(x: SEXP) -> SEXP;
        pub fn Rf_mkCharLen(s: *const c_char, n: c_int) -> SEXP;
        pub fn Rf_asInteger(x: SEXP) -> c_int;
        pub fn Rf_protect(x: SEXP) -> SEXP;
        pub fn Rf_unprotect(n: c_int);
        pub fn Rf_xlength(x: SEXP) -> R_xlen_t;
        pub fn R_CHAR(x: SEXP) -> *const c_char;
        pub fn STRING_ELT(x: SEXP, i: R_xlen_t) -> SEXP;
        pub fn RAW(x: SEXP) -> *mut u8;
        pub static R_NilValue: SEXP;
    }
}

use rapi::SEXP;

/// Process-global handle registry. R refers to a worker by `i32` id; the
/// `RWorker` (and its socket) never leave Rust.
fn registry() -> &'static Mutex<HashMap<i32, RWorker>> {
    static REG: OnceLock<Mutex<HashMap<i32, RWorker>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_ID: AtomicI32 = AtomicI32::new(1);

const OK: c_int = 0;
const ERR: c_int = 1;

// ── SEXP marshalling helpers ────────────────────────────────────────────

/// First element of an R character vector → owned `String`.
unsafe fn sexp_to_string(s: SEXP) -> Option<String> {
    if s == unsafe { rapi::R_NilValue } || unsafe { rapi::Rf_xlength(s) } < 1 {
        return None;
    }
    let elt = unsafe { rapi::STRING_ELT(s, 0) };
    let cptr = unsafe { rapi::R_CHAR(elt) };
    if cptr.is_null() {
        return None;
    }
    Some(
        unsafe { std::ffi::CStr::from_ptr(cptr) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// `&str` → R character scalar (`STRSXP` of length 1).
unsafe fn string_to_sexp(s: &str) -> SEXP {
    let ch = unsafe {
        rapi::Rf_protect(rapi::Rf_mkCharLen(
            s.as_ptr() as *const c_char,
            s.len() as c_int,
        ))
    };
    let out = unsafe { rapi::Rf_ScalarString(ch) };
    unsafe { rapi::Rf_unprotect(1) };
    out
}

/// `&[u8]` → R `RAWSXP`.
unsafe fn bytes_to_raw(b: &[u8]) -> SEXP {
    let out = unsafe {
        rapi::Rf_protect(rapi::Rf_allocVector(
            rapi::RAWSXP,
            b.len() as rapi::R_xlen_t,
        ))
    };
    if !b.is_empty() {
        let dst = unsafe { rapi::RAW(out) };
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), dst, b.len()) };
    }
    unsafe { rapi::Rf_unprotect(1) };
    out
}

/// An R `RAWSXP` → owned `Vec<u8>`.
unsafe fn raw_to_bytes(s: SEXP) -> Vec<u8> {
    let len = unsafe { rapi::Rf_xlength(s) } as usize;
    if len == 0 {
        return Vec::new();
    }
    unsafe { slice::from_raw_parts(rapi::RAW(s), len) }.to_vec()
}

unsafe fn nil() -> SEXP {
    unsafe { rapi::R_NilValue }
}

// ── R `.Call` entry points ──────────────────────────────────────────────

/// `r_listen(path)` → integer handle id (`-1` on failure).
///
/// # Safety
/// Called by R via `.Call`; `path` is an R character vector.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_listen(path: SEXP) -> SEXP {
    let path = match unsafe { sexp_to_string(path) } {
        Some(p) => p,
        None => return unsafe { rapi::Rf_ScalarInteger(-1) },
    };
    match RWorker::listen(&path) {
        Ok(worker) => {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            registry().lock().unwrap().insert(id, worker);
            unsafe { rapi::Rf_ScalarInteger(id) }
        }
        Err(e) => {
            eprintln!("eigenius-r-worker: listen({path}) failed: {e}");
            unsafe { rapi::Rf_ScalarInteger(-1) }
        }
    }
}

/// `r_accept_next(id)` → integer status (`0` ok, `-1` error). Accepts the
/// next substrate connection on the bound listener, replacing the current
/// stream. The substrate dials a fresh connection per RPC, so the driver
/// calls this after a connection closes (`RequestKind::Closed`).
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_accept_next(id: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    let mut reg = registry().lock().unwrap();
    let rc = match reg.get_mut(&id) {
        Some(w) => match w.accept_next() {
            Ok(()) => OK,
            Err(e) => {
                eprintln!("eigenius-r-worker: accept_next failed: {e}");
                -1
            }
        },
        None => -1,
    };
    unsafe { rapi::Rf_ScalarInteger(rc) }
}

/// `r_next_kind(id)` → integer [`crate::RequestKind`].
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_next_kind(id: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    let mut reg = registry().lock().unwrap();
    let kind = match reg.get_mut(&id) {
        Some(w) => w.next_request() as c_int,
        None => crate::RequestKind::TransportError as c_int,
    };
    unsafe { rapi::Rf_ScalarInteger(kind) }
}

/// `r_invocation_id(id)` → character scalar, or `NULL`.
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_invocation_id(id: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    let reg = registry().lock().unwrap();
    match reg.get(&id).and_then(|w| w.invocation_id()) {
        Some(s) => unsafe { string_to_sexp(s) },
        None => unsafe { nil() },
    }
}

/// `r_script_source(id)` → character scalar, or `NULL`.
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_script_source(id: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    let reg = registry().lock().unwrap();
    match reg.get(&id).and_then(|w| w.script_source()) {
        Some(s) => unsafe { string_to_sexp(s) },
        None => unsafe { nil() },
    }
}

/// `r_input_count(id)` → integer (`-1` if no such worker).
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_input_count(id: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    let reg = registry().lock().unwrap();
    let n = reg
        .get(&id)
        .map(|w| w.inputs().len() as c_int)
        .unwrap_or(-1);
    unsafe { rapi::Rf_ScalarInteger(n) }
}

/// `r_input(id, idx)` → `RAWSXP` of the `idx`-th input, or `NULL`.
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_input(id: SEXP, idx: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    let idx = unsafe { rapi::Rf_asInteger(idx) } as usize;
    let reg = registry().lock().unwrap();
    match reg.get(&id).and_then(|w| w.inputs().get(idx)) {
        Some(b) => unsafe { bytes_to_raw(b) },
        None => unsafe { nil() },
    }
}

/// `r_send_health(id)` → integer status (`0` ok). Default cross-check
/// fields for P1.2; the pinned-image cross-check + numerical metadata
/// land in P3.
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_send_health(id: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    let mut reg = registry().lock().unwrap();
    let rc = match reg.get_mut(&id) {
        Some(w) => w.send_health(HealthInfo::default()).map_or(ERR, |()| OK),
        None => ERR,
    };
    unsafe { rapi::Rf_ScalarInteger(rc) }
}

/// `r_send_dispatch_ok(id, out)` → integer status. `out` is a `RAWSXP`
/// (the CBOR-encoded Eigon `DerivedResource`).
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_send_dispatch_ok(id: SEXP, out: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    let output = unsafe { raw_to_bytes(out) };
    let mut reg = registry().lock().unwrap();
    let rc = match reg.get_mut(&id) {
        Some(w) => w
            .send_dispatch_ok(output, Vec::new(), None)
            .map_or(ERR, |()| OK),
        None => ERR,
    };
    unsafe { rapi::Rf_ScalarInteger(rc) }
}

/// `r_send_dispatch_failed(id, kind, msg)` → integer status.
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_send_dispatch_failed(id: SEXP, kind: SEXP, msg: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    let kind = unsafe { sexp_to_string(kind) }.unwrap_or_else(|| "runtime_error".to_string());
    let msg = unsafe { sexp_to_string(msg) }.unwrap_or_default();
    let mut reg = registry().lock().unwrap();
    let rc = match reg.get_mut(&id) {
        Some(w) => w.send_dispatch_failed(kind, msg).map_or(ERR, |()| OK),
        None => ERR,
    };
    unsafe { rapi::Rf_ScalarInteger(rc) }
}

/// `r_send_evicted(id)` → integer status.
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_send_evicted(id: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    let mut reg = registry().lock().unwrap();
    let rc = match reg.get_mut(&id) {
        Some(w) => w.send_evicted().map_or(ERR, |()| OK),
        None => ERR,
    };
    unsafe { rapi::Rf_ScalarInteger(rc) }
}

/// `r_close(id)` → `NULL`. Drops the worker (closes its socket).
///
/// # Safety
/// Called by R via `.Call`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn r_close(id: SEXP) -> SEXP {
    let id = unsafe { rapi::Rf_asInteger(id) };
    registry().lock().unwrap().remove(&id);
    unsafe { nil() }
}
