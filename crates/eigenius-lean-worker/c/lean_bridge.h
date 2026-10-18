/*
 * Copyright 2026 The Eigenius Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
 * implied. See the License for the specific language governing
 * permissions and limitations under the License.
 */

/*
 * Thunks re-exposing lean.h's `static inline` functions as proper
 * linkable C symbols Rust can `extern "C"` declare. Only the
 * functions the worker's FFI bridge actually uses are thunked —
 * we keep the surface minimal so future Lean toolchain bumps (which
 * may rename/refactor inline helpers) touch as little of our code
 * as possible.
 *
 * Naming: every thunk is prefixed with `ei_` (for "eigenius") to
 * avoid colliding with Lean's own symbols if a future Lean version
 * promotes one of the inlines to a real symbol.
 */
#ifndef EIGENIUS_LEAN_BRIDGE_H
#define EIGENIUS_LEAN_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

/*
 * Lean object pointer + external class are opaque to anyone using
 * this header — we never dereference them outside `lean_bridge.c`,
 * which has the real `<lean/lean.h>` definitions in scope. Aliasing
 * to `void` sidesteps the typedef collision: lean.h declares
 * `lean_object` as a typedef of an *anonymous* struct, so we can't
 * forward-declare it as `struct lean_object` here without C seeing
 * two distinct types.
 *
 * `EiLeanObj` / `EiLeanExternalClass` are the names this header
 * exposes; the implementation file casts between them and the real
 * Lean types.
 */
typedef void EiLeanObj;
typedef void EiLeanExternalClass;

/* Function-pointer types matching Lean's external-object callbacks
 * (lean.h's `lean_external_finalize_proc` and
 * `lean_external_foreach_proc`). Repeated here so callers of
 * `ei_lean_register_external_class` can declare conforming
 * callbacks without including `<lean/lean.h>`. */
typedef void (*EiLeanFinalizeProc)(void *);
typedef void (*EiLeanForeachProc)(void *, EiLeanObj *);

#ifdef __cplusplus
extern "C" {
#endif

/* --- ByteArray (sarray<u8>) operations ------------------------- */

/* Allocate a Lean ByteArray (sarray<u8>) of the given size +
 * capacity. Returns a heap-allocated Lean object the caller
 * owns. */
EiLeanObj *ei_lean_alloc_byte_array(size_t size, size_t capacity);

/* Get the size of a Lean ByteArray. The argument is borrowed (Lean
 * retains ownership; we don't increment the refcount). */
size_t ei_lean_sarray_size(EiLeanObj *arr);

/* Get the raw byte pointer for a Lean ByteArray. Same borrow
 * discipline as `ei_lean_sarray_size`. */
uint8_t *ei_lean_sarray_cptr(EiLeanObj *arr);

/* --- IO results ------------------------------------------------- */

/* Wrap a value in `Except.ok` (used as `EStateM.Result.ok` on the
 * IO monad's `Except`-style return). */
EiLeanObj *ei_lean_io_result_mk_ok(EiLeanObj *val);

/* Wrap an error value in `Except.error`. */
EiLeanObj *ei_lean_io_result_mk_error(EiLeanObj *err);

/* The IO "world" token — `lean_box(0)`. Some FFI shapes carry it
 * through arguments; we expose it so the bridge can construct one
 * deterministically. */
EiLeanObj *ei_lean_io_mk_world(void);

/* --- Scalars (box / unbox) ------------------------------------- */

/* Box a small unsigned integer as a Lean object (tagged-pointer
 * representation). Used for `Bool` / `UInt8` / `UInt32` / etc.
 * (anything that fits in a tagged pointer). */
EiLeanObj *ei_lean_box(size_t n);

/* Unbox a scalar Lean object. Caller asserts the object is a
 * boxed small integer (the kind returned by `ei_lean_box`). */
size_t ei_lean_unbox(EiLeanObj *o);

/* Box a `USize` value. Lean's `USize` is 64-bit on 64-bit
 * platforms and doesn't fit in a tagged pointer; the boxed
 * representation is a heap-allocated constructor with the scalar
 * stored at offset 0 (zero object fields, 8-byte scalar payload).
 *
 * Required for `IO USize` extern returns: Lean's generated
 * unwrapping code does `lean_ctor_get_usize(o, 0)` which expects
 * the heap-ctor layout, *not* a tagged pointer. */
EiLeanObj *ei_lean_box_usize(size_t v);

/* --- External objects (opaque-handle pattern) ------------------ */

/* Register a new external-object class. Lean's GC calls the
 * `finalize` callback when the wrapping `lean_object` is collected.
 * Wraps lean.h's `lean_register_external_class` (which is already
 * a real linkable symbol, but we re-expose with our typedef shapes
 * so the Rust side never sees `lean_object` direct). */
EiLeanExternalClass *ei_lean_register_external_class(
    EiLeanFinalizeProc finalize, EiLeanForeachProc foreach);

/* Wrap a raw pointer as a Lean "external object" of the given
 * class. Lean's GC will eventually call the class's
 * `finalize` callback to release the underlying allocation. */
EiLeanObj *ei_lean_alloc_external(EiLeanExternalClass *cls, void *data);

/* Extract the raw pointer from a Lean external object. The caller
 * is responsible for asserting class identity (Lean's FFI ABI
 * doesn't check). */
void *ei_lean_get_external_data(EiLeanObj *o);

#ifdef __cplusplus
}
#endif

#endif /* EIGENIUS_LEAN_BRIDGE_H */
